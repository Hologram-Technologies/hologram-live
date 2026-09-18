// A streaming zip writer for the browser and Node: STORE only (model weights do not compress), zip64 when a file or
// the archive passes 4 GB, and every entry written as it arrives so nothing is held in memory. No dependencies.
//
//   const zip = new ZipWriter((bytes) => sink.write(bytes));
//   await zip.add("path/in/zip", size, readableStream, (chunk) => hasher.update(chunk));
//   await zip.finish();

const encoder = new TextEncoder();
const CRC_TABLE = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});
function crc32(crc, bytes) {
  let c = ~crc;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return ~c;
}

const LIMIT32 = 0xffffffff;
const u16 = (v) => [v & 0xff, (v >>> 8) & 0xff];
const u32 = (v) => [v & 0xff, (v >>> 8) & 0xff, (v >>> 16) & 0xff, (v >>> 24) & 0xff];
const u64 = (v) => { const b = BigInt(v); return [...u32(Number(b & 0xffffffffn)), ...u32(Number(b >> 32n))]; };
function dosStamp(d = new Date()) {
  return { time: (d.getHours() << 11) | (d.getMinutes() << 5) | (d.getSeconds() >> 1), date: ((Math.max(1980, d.getFullYear()) - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate() };
}

export class ZipWriter {
  // The exact byte length of the archive for `entries` ([{ name, size }], in the order they will be added). STORE
  // only, so every part is a fixed function of names and sizes; a download can declare Content-Length before the
  // first byte. Must mirror add() and finish() exactly: qa/zip-size.test.mjs holds the two together.
  static sizeOf(entries) {
    let offset = 0, directory = 0, any64 = false;
    for (const e of entries) {
      const name = encoder.encode(e.name).length, zip64 = e.size >= LIMIT32, bigOffset = offset >= LIMIT32;
      any64 ||= zip64;
      directory += 46 + name + (zip64 || bigOffset ? 4 + (zip64 ? 16 : 0) + (bigOffset ? 8 : 0) : 0);
      offset += 30 + name + (zip64 ? 20 : 0) + e.size + (zip64 ? 24 : 16);
    }
    const needs64 = entries.length >= 0xffff || offset >= LIMIT32 || directory >= LIMIT32 || any64;
    return offset + directory + (needs64 ? 56 + 20 : 0) + 22;
  }

  constructor(write) {
    this.write = write;
    this.offset = 0;
    this.entries = [];
  }

  async put(bytes) {
    const u8 = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
    await this.write(u8);
    this.offset += u8.length;
  }

  // `stream` is a ReadableStream of bytes; `onChunk` sees every chunk (for hashing). `size` must be exact.
  async add(name, size, stream, onChunk) {
    const nameBytes = encoder.encode(name), { time, date } = dosStamp();
    const zip64 = size >= LIMIT32, start = this.offset;
    const extra = zip64 ? [...u16(0x0001), ...u16(16), ...u64(size), ...u64(size)] : [];
    // Flag bit 3: sizes and CRC follow the data in a descriptor, because the CRC is only known after streaming.
    await this.put([...u32(0x04034b50), ...u16(zip64 ? 45 : 20), ...u16(0x0808), ...u16(0), ...u16(time), ...u16(date),
      ...u32(0), ...u32(zip64 ? LIMIT32 : 0), ...u32(zip64 ? LIMIT32 : 0), ...u16(nameBytes.length), ...u16(extra.length), ...nameBytes, ...extra]);
    let crc = 0, written = 0;
    const reader = stream.getReader();
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      crc = crc32(crc, value);
      written += value.length;
      onChunk?.(value);
      await this.put(value);
    }
    if (written !== size) throw new Error(`${name}: expected ${size} bytes, received ${written}`);
    crc >>>= 0;
    await this.put([...u32(0x08074b50), ...u32(crc), ...(zip64 ? [...u64(size), ...u64(size)] : [...u32(size), ...u32(size)])]);
    this.entries.push({ nameBytes, size, crc, start, time, date, zip64 });
  }

  async finish() {
    const directoryStart = this.offset;
    for (const e of this.entries) {
      const bigOffset = e.start >= LIMIT32, needs64 = e.zip64 || bigOffset;
      const extra = needs64 ? [...u16(0x0001), ...u16((e.zip64 ? 16 : 0) + (bigOffset ? 8 : 0)), ...(e.zip64 ? [...u64(e.size), ...u64(e.size)] : []), ...(bigOffset ? u64(e.start) : [])] : [];
      await this.put([...u32(0x02014b50), ...u16(45), ...u16(needs64 ? 45 : 20), ...u16(0x0808), ...u16(0), ...u16(e.time), ...u16(e.date),
        ...u32(e.crc), ...u32(e.zip64 ? LIMIT32 : e.size), ...u32(e.zip64 ? LIMIT32 : e.size), ...u16(e.nameBytes.length), ...u16(extra.length),
        ...u16(0), ...u16(0), ...u16(0), ...u32(0), ...u32(bigOffset ? LIMIT32 : e.start), ...e.nameBytes, ...extra]);
    }
    const directorySize = this.offset - directoryStart, count = this.entries.length;
    if (count >= 0xffff || directoryStart >= LIMIT32 || directorySize >= LIMIT32 || this.entries.some((e) => e.zip64)) {
      const eocd64 = this.offset;
      await this.put([...u32(0x06064b50), ...u64(44), ...u16(45), ...u16(45), ...u32(0), ...u32(0), ...u64(count), ...u64(count), ...u64(directorySize), ...u64(directoryStart)]);
      await this.put([...u32(0x07064b50), ...u32(0), ...u64(eocd64), ...u32(1)]);
    }
    await this.put([...u32(0x06054b50), ...u16(0), ...u16(0), ...u16(Math.min(count, 0xffff)), ...u16(Math.min(count, 0xffff)),
      ...u32(Math.min(directorySize, LIMIT32)), ...u32(Math.min(directoryStart, LIMIT32)), ...u16(0)]);
  }
}
