// A restricted pickle reader for torch checkpoints. It never runs code: GLOBAL and REDUCE resolve only against
// the allowlist below, and anything else throws. It returns the object graph with tensors as plain records
// (storage key, dtype, offset, size, stride), which is all the tensor index needs.
//
// Allowlist (the whole defence, since pickle is arbitrary code by design):
//   torch._utils._rebuild_tensor_v2, torch._utils._rebuild_parameter, collections.OrderedDict, torch.Size,
//   and the torch storage classes (as type markers only; calling one throws).
// persistent ids accepted: ('storage', type, key, location, numel) and the legacy six-field form whose last
// field (view metadata) is None.

export const STORAGE = {
  FloatStorage: ["F32", 4], HalfStorage: ["F16", 2], BFloat16Storage: ["BF16", 2], DoubleStorage: ["F64", 8],
  LongStorage: ["I64", 8], IntStorage: ["I32", 4], ShortStorage: ["I16", 2], CharStorage: ["I8", 1],
  ByteStorage: ["U8", 1], BoolStorage: ["BOOL", 1],
};

class StorageType { constructor(name) { this.name = name; [this.dtype, this.itemsize] = STORAGE[name]; } }
export class StorageRef { constructor(key, type, numel) { this.key = String(key); this.dtype = type.dtype; this.itemsize = type.itemsize; this.numel = Number(numel); } }
export class TensorRec {
  constructor(storage, offset, size, stride) { this.storage = storage; this.offset = Number(offset); this.size = size.map(Number); this.stride = stride.map(Number); }
}
class OrderedDictObj { constructor() { this.map = new Map(); } }
const MARK = Symbol("mark");

const ALLOW = new Map([
  ["torch._utils._rebuild_tensor_v2", (storage, offset, size, stride) => {
    if (!(storage instanceof StorageRef)) throw new Error("rebuild without a storage ref");
    return new TensorRec(storage, offset, size, stride);
  }],
  ["torch._utils._rebuild_parameter", (data) => data],
  ["collections.OrderedDict", () => new OrderedDictObj()],
  ["torch.Size", (t) => t],
]);
for (const s of Object.keys(STORAGE)) ALLOW.set(`torch.${s}`, new StorageType(s));

// Parse one pickle starting at `pos` in `buf`. Returns { value, end } where end is the offset after STOP.
export function unpickle(buf, pos = 0) {
  const stack = [], memo = new Map(), used = new Set();
  let i = pos;
  const u8 = () => buf[i++];
  const u16 = () => { const v = buf.readUInt16LE(i); i += 2; return v; };
  const u32 = () => { const v = buf.readUInt32LE(i); i += 4; return v; };
  const i32 = () => { const v = buf.readInt32LE(i); i += 4; return v; };
  const bytes = (n) => { if (i + n > buf.length) throw new RangeError("pickle truncated"); const b = buf.subarray(i, i + n); i += n; return b; };
  const line = () => { const j = buf.indexOf(10, i); if (j < 0) throw new RangeError("pickle truncated"); const s = buf.toString("latin1", i, j); i = j + 1; return s; };
  const popMark = () => { const k = stack.lastIndexOf(MARK); if (k < 0) throw new Error("no mark"); const items = stack.splice(k + 1); stack.pop(); return items; };
  const find = (mod, name) => {
    const k = `${mod}.${name}`;
    if (!ALLOW.has(k)) throw new Error(`refused global ${k}`);
    used.add(k); return ALLOW.get(k);
  };
  const setitem = (d, k, v) => {
    if (d instanceof OrderedDictObj) d.map.set(k, v);
    else if (d instanceof Map) d.set(k, v);
    else throw new Error("setitem on non-dict");
  };
  for (;;) {
    if (i >= buf.length) throw new RangeError("pickle truncated");
    const op = u8();
    switch (op) {
      case 0x80: u8(); break;                                   // PROTO
      case 0x95: i += 8; break;                                 // FRAME
      case 0x2e: return { value: stack.pop(), end: i, used };   // STOP
      case 0x28: stack.push(MARK); break;                       // MARK
      case 0x4e: stack.push(null); break;                       // NONE
      case 0x88: stack.push(true); break; case 0x89: stack.push(false); break;
      case 0x4a: stack.push(i32()); break;                      // BININT
      case 0x4b: stack.push(u8()); break;                       // BININT1
      case 0x4d: stack.push(u16()); break;                      // BININT2
      case 0x8a: { const n = u8(); const b = bytes(n); let v = 0n; for (let k = n - 1; k >= 0; k--) v = (v << 8n) | BigInt(b[k]); if (n && b[n - 1] & 0x80) v -= 1n << BigInt(8 * n); stack.push(v); break; } // LONG1
      case 0x47: { const v = buf.readDoubleBE(i); i += 8; stack.push(v); break; } // BINFLOAT
      case 0x58: { const n = u32(); stack.push(bytes(n).toString("utf8")); break; } // BINUNICODE
      case 0x8c: { const n = u8(); stack.push(bytes(n).toString("utf8")); break; }  // SHORT_BINUNICODE
      case 0x54: { const n = u32(); stack.push(bytes(n).toString("latin1")); break; } // BINSTRING
      case 0x55: { const n = u8(); stack.push(bytes(n).toString("latin1")); break; }  // SHORT_BINSTRING
      case 0x42: { const n = u32(); stack.push(Buffer.from(bytes(n))); break; }      // BINBYTES
      case 0x43: { const n = u8(); stack.push(Buffer.from(bytes(n))); break; }       // SHORT_BINBYTES
      case 0x7d: stack.push(new Map()); break;                  // EMPTY_DICT
      case 0x5d: stack.push([]); break;                         // EMPTY_LIST
      case 0x29: stack.push([]); break;                         // EMPTY_TUPLE
      case 0x74: stack.push(popMark()); break;                  // TUPLE
      case 0x85: stack.push([stack.pop()]); break;              // TUPLE1
      case 0x86: { const b = stack.pop(), a = stack.pop(); stack.push([a, b]); break; }
      case 0x87: { const c = stack.pop(), b = stack.pop(), a = stack.pop(); stack.push([a, b, c]); break; }
      case 0x6c: stack.push(popMark()); break;                  // LIST
      case 0x61: { const v = stack.pop(); stack[stack.length - 1].push(v); break; }      // APPEND
      case 0x65: { const items = popMark(); stack[stack.length - 1].push(...items); break; } // APPENDS
      case 0x64: { const items = popMark(); const d = new Map(); for (let k = 0; k < items.length; k += 2) d.set(items[k], items[k + 1]); stack.push(d); break; } // DICT
      case 0x73: { const v = stack.pop(), k = stack.pop(); setitem(stack[stack.length - 1], k, v); break; } // SETITEM
      case 0x75: { const items = popMark(); const d = stack[stack.length - 1]; for (let k = 0; k < items.length; k += 2) setitem(d, items[k], items[k + 1]); break; } // SETITEMS
      case 0x71: memo.set(u8(), stack[stack.length - 1]); break;   // BINPUT
      case 0x72: memo.set(u32(), stack[stack.length - 1]); break;  // LONG_BINPUT
      case 0x94: memo.set(memo.size, stack[stack.length - 1]); break; // MEMOIZE
      case 0x68: stack.push(memo.get(u8())); break;                // BINGET
      case 0x6a: stack.push(memo.get(u32())); break;               // LONG_BINGET
      case 0x63: { const mod = line(), name = line(); stack.push(find(mod, name)); break; } // GLOBAL
      case 0x93: { const name = stack.pop(), mod = stack.pop(); stack.push(find(mod, name)); break; } // STACK_GLOBAL
      case 0x52: { const args = stack.pop(), fn = stack.pop();   // REDUCE
        if (typeof fn !== "function") throw new Error("REDUCE on a non-callable");
        stack.push(fn(...args)); break; }
      case 0x81: { stack.pop(); stack.pop(); throw new Error("refused NEWOBJ"); }
      case 0x62: { const state = stack.pop(); const obj = stack[stack.length - 1]; // BUILD: only metadata on dicts, ignored
        if (!(obj instanceof OrderedDictObj || obj instanceof Map || obj instanceof TensorRec)) throw new Error("refused BUILD");
        void state; break; }
      case 0x51: { const pid = stack.pop();                       // BINPERSID
        const ok = Array.isArray(pid) && pid[0] === "storage" && (pid.length === 5 || (pid.length === 6 && pid[5] === null)) && pid[1] instanceof StorageType;
        if (!ok) throw new Error("refused persistent id");
        stack.push(new StorageRef(pid[2], pid[1], pid[4])); break; }
      default: throw new Error(`refused opcode 0x${op.toString(16)} at ${i - 1}`);
    }
  }
}

// Flatten the loaded object into name -> TensorRec (state dicts, nested dicts, OrderedDicts).
export function flatten(obj, prefix = "", out = new Map()) {
  if (obj instanceof TensorRec) { out.set(prefix, obj); return out; }
  const entries = obj instanceof OrderedDictObj ? obj.map : obj instanceof Map ? obj : null;
  if (!entries) return out;
  for (const [k, v] of entries) {
    if (typeof k !== "string" || k === "_metadata") continue;
    flatten(v, prefix ? `${prefix}.${k}` : k, out);
  }
  return out;
}

export function contiguous(size, stride) {
  let acc = 1;
  for (let d = size.length - 1; d >= 0; d--) { if (size[d] !== 1 && stride[d] !== acc) return false; acc *= size[d]; }
  return true;
}
