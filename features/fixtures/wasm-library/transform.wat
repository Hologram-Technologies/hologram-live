;; Guest contract v1 fixture (see src/holo_wasm.rs): exports `memory`,
;; `holo_alloc(len) -> ptr`, and `holo_run(ptr, len) -> out_ptr << 32 | out_len`.
;; The transform is ASCII uppercasing, so tests can assert on the output bytes.
;;
;; The heap bump-allocates from offset 1024 and never reclaims: every run uses
;; a fresh instance, so residency does not accumulate guest state.
(module
  (memory $memory 1)
  (export "memory" (memory $memory))
  (global $heap (mut i32) (i32.const 1024))

  (func $holo_alloc (export "holo_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (local.get $ptr) (local.get $len)))
    (local.get $ptr))

  (func $holo_run (export "holo_run") (param $ptr i32) (param $len i32) (result i64)
    (local $out i32)
    (local $i i32)
    (local $byte i32)
    (local.set $out (call $holo_alloc (local.get $len)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (local.set $byte (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        ;; 'a'..='z' map to 'A'..='Z'; every other byte passes through.
        (if (i32.and
              (i32.ge_u (local.get $byte) (i32.const 97))
              (i32.le_u (local.get $byte) (i32.const 122)))
          (then (local.set $byte (i32.sub (local.get $byte) (i32.const 32)))))
        (i32.store8 (i32.add (local.get $out) (local.get $i)) (local.get $byte))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
      (i64.extend_i32_u (local.get $len)))))
