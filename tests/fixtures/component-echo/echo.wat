(component
  (core module $main (;0;)
    (type (;0;) (func (param i32 i32)))
    (type (;1;) (func (param i32 i32 i32) (result i32)))
    (type (;2;) (func (param i32 i32) (result i32)))
    (type (;3;) (func))
    (type (;4;) (func (param i32)))
    (type (;5;) (func (param i32 i32 i32)))
    (type (;6;) (func (param i32 i32 i32 i32) (result i32)))
    (type (;7;) (func (param i32 i32 i32 i32 i32)))
    (type (;8;) (func (param i32 i32 i32 i32 i32 i32)))
    (type (;9;) (func (param i32) (result i32)))
    (table (;0;) 20 20 funcref)
    (memory (;0;) 17)
    (global $__stack_pointer (;0;) (mut i32) i32.const 1048576)
    (export "memory" (memory 0))
    (export "cabi_post_hologram:application/guest@1.0.0#run" (func $cabi_post_hologram:application/guest@1.0.0#run))
    (export "hologram:application/guest@1.0.0#run" (func $hologram:application/guest@1.0.0#run))
    (export "cabi_realloc" (func $cabi_realloc))
    (export "cabi_realloc_wit_bindgen_0_57_1" (func $_RNvNtCs9oOC0MOPQlZ_11wit_bindgen2rt12cabi_realloc))
    (elem (;0;) (i32.const 1) func $_RNvCseJA7PpHSr1U_31hologram_component_echo_fixture40___link_custom_section_describing_imports $cabi_realloc $_RNvNtCsdl5sGgnNXvY_3std5alloc24default_alloc_error_hook $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueNtNtCsewWLk9TkM7w_5alloc6string6StringECsdl5sGgnNXvY_3std $_RNvXsZ_NtCsewWLk9TkM7w_5alloc6stringNtB5_6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write9write_str $_RNvXsZ_NtCsewWLk9TkM7w_5alloc6stringNtB5_6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write10write_char $_RNvYNtNtCsewWLk9TkM7w_5alloc6string6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write9write_fmtCsdl5sGgnNXvY_3std $_RNvXs2_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core3fmt7Display3fmt $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload8take_box $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload3get $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload6as_str $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueNtNvNtCsdl5sGgnNXvY_3std9panicking13panic_handler19FormatStringPayloadEBH_ $_RNvXs0_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core3fmt7Display3fmt $_RNvXs_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB4_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload8take_box $_RNvXs_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB4_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload3get $_RNvYINtNvNtCsdl5sGgnNXvY_3std9panicking11begin_panic7PayloadReENtNtCsdkdt1aaAg1T_4core5panic12PanicPayload6as_strB9_ $_RNvXNtCsdkdt1aaAg1T_4core3anyReNtB2_3Any7type_idCsdl5sGgnNXvY_3std $_RNvXNtCsdkdt1aaAg1T_4core3anyNtNtCsewWLk9TkM7w_5alloc6string6StringNtB2_3Any7type_idCsdl5sGgnNXvY_3std $_RNvXs1i_NtCsdkdt1aaAg1T_4core3fmtReNtB6_7Display3fmtB8_)
    (func $__wasm_call_ctors (;0;) (type 3))
    (func $_RNvCseJA7PpHSr1U_31hologram_component_echo_fixture40___link_custom_section_describing_imports (;1;) (type 3))
    (func $cabi_post_hologram:application/guest@1.0.0#run (;2;) (type 4) (param i32)
      (local i32 i32)
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 0
            i32.load8_u
            br_if 0 (;@3;)
            local.get 0
            i32.load offset=8
            local.tee 1
            i32.eqz
            br_if 2 (;@1;)
            i32.const 4
            local.set 2
            br 1 (;@2;)
          end
          local.get 0
          i32.load offset=12
          local.tee 1
          i32.eqz
          br_if 1 (;@1;)
          i32.const 8
          local.set 2
        end
        local.get 0
        local.get 2
        i32.add
        i32.load
        local.get 1
        i32.const 1
        call $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc
      end
    )
    (func $hologram:application/guest@1.0.0#run (;3;) (type 2) (param i32 i32) (result i32)
      call $_RNvNtCs9oOC0MOPQlZ_11wit_bindgen2rt14run_ctors_once
      block ;; label = @1
        block ;; label = @2
          local.get 1
          i32.const 11
          i32.ne
          br_if 0 (;@2;)
          local.get 0
          i64.load align=1
          i64.const 8243044671147046247
          i64.xor
          local.get 0
          i32.const 3
          i32.add
          i64.load align=1
          i64.const 8245935278387983475
          i64.xor
          i64.or
          i64.const 0
          i64.ne
          br_if 0 (;@2;)
          call $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2
          i32.const 15
          i32.const 1
          call $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc
          local.tee 1
          i32.eqz
          br_if 1 (;@1;)
          local.get 1
          i32.const 0
          i64.load offset=1048587 align=1
          i64.store offset=7 align=1
          local.get 1
          i32.const 0
          i64.load offset=1048580 align=1
          i64.store align=1
          local.get 0
          i32.const 11
          i32.const 1
          call $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc
          i32.const 0
          i32.const 15
          i32.store offset=1049240
          i32.const 0
          i32.const 1
          i32.store8 offset=1049232
          i32.const 0
          i32.const 1
          i32.store8 offset=1049228
          i32.const 0
          local.get 1
          i32.store offset=1049236
          i32.const 1049228
          return
        end
        i32.const 0
        local.get 1
        i32.store offset=1049236
        i32.const 0
        local.get 0
        i32.store offset=1049232
        i32.const 0
        i32.const 0
        i32.store8 offset=1049228
        i32.const 1049228
        return
      end
      i32.const 1
      i32.const 15
      call $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec12handle_error
      unreachable
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc (;4;) (type 2) (param i32 i32) (result i32)
      local.get 0
      local.get 1
      call $_RNvCs9wFQrvczXsK_7___rustc11___rdl_alloc
      return
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc (;5;) (type 5) (param i32 i32 i32)
      local.get 0
      local.get 1
      local.get 2
      call $_RNvCs9wFQrvczXsK_7___rustc13___rdl_dealloc
      return
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc14___rust_realloc (;6;) (type 6) (param i32 i32 i32 i32) (result i32)
      local.get 0
      local.get 1
      local.get 2
      local.get 3
      call $_RNvCs9wFQrvczXsK_7___rustc13___rdl_realloc
      return
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2 (;7;) (type 3)
      return
    )
    (func $_RNvNtCs9oOC0MOPQlZ_11wit_bindgen2rt12cabi_realloc (;8;) (type 6) (param i32 i32 i32 i32) (result i32)
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 1
            br_if 0 (;@3;)
            local.get 3
            i32.eqz
            br_if 2 (;@1;)
            call $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2
            local.get 3
            local.get 2
            call $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc
            local.set 2
            br 1 (;@2;)
          end
          local.get 0
          local.get 1
          local.get 2
          local.get 3
          call $_RNvCs9wFQrvczXsK_7___rustc14___rust_realloc
          local.set 2
        end
        local.get 2
        br_if 0 (;@1;)
        unreachable
      end
      local.get 2
    )
    (func $_RNvNtCs9oOC0MOPQlZ_11wit_bindgen2rt14run_ctors_once (;9;) (type 3)
      block ;; label = @1
        i32.const 0
        i32.load8_u offset=1049244
        br_if 0 (;@1;)
        call $__wasm_call_ctors
        i32.const 0
        i32.const 1
        i32.store8 offset=1049244
      end
    )
    (func $cabi_realloc (;10;) (type 6) (param i32 i32 i32 i32) (result i32)
      local.get 0
      local.get 1
      local.get 2
      local.get 3
      call $_RNvNtCs9oOC0MOPQlZ_11wit_bindgen2rt12cabi_realloc
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc18___rust_start_panic (;11;) (type 2) (param i32 i32) (result i32)
      call $_RNvCs9wFQrvczXsK_7___rustc12___rust_abort
      unreachable
    )
    (func $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueINtNtB4_6option6OptionINtNtCsewWLk9TkM7w_5alloc3vec3VechEEECsdl5sGgnNXvY_3std (;12;) (type 0) (param i32 i32)
      block ;; label = @1
        local.get 0
        i32.const -1
        i32.add
        i32.const -2
        i32.ge_u
        br_if 0 (;@1;)
        local.get 1
        local.get 0
        i32.const 1
        call $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc
      end
    )
    (func $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueNtNtCsewWLk9TkM7w_5alloc6string6StringECsdl5sGgnNXvY_3std (;13;) (type 4) (param i32)
      (local i32)
      block ;; label = @1
        local.get 0
        i32.load
        local.tee 1
        i32.eqz
        br_if 0 (;@1;)
        local.get 0
        i32.load offset=4
        local.get 1
        i32.const 1
        call $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc
      end
    )
    (func $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueNtNvNtCsdl5sGgnNXvY_3std9panicking13panic_handler19FormatStringPayloadEBH_ (;14;) (type 4) (param i32)
      (local i32)
      block ;; label = @1
        local.get 0
        i32.load
        local.tee 1
        i32.const 1
        i32.lt_s
        br_if 0 (;@1;)
        local.get 0
        i32.load offset=4
        local.get 1
        i32.const 1
        call $_RNvCs9wFQrvczXsK_7___rustc14___rust_dealloc
      end
    )
    (func $_RINvNvMs2_NtCsewWLk9TkM7w_5alloc7raw_vecINtB8_11RawVecInnerpE7reserve21do_reserve_and_handleNtNtBa_5alloc6GlobalECsdl5sGgnNXvY_3std (;15;) (type 7) (param i32 i32 i32 i32 i32)
      (local i32)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 5
      global.set $__stack_pointer
      block ;; label = @1
        local.get 2
        local.get 1
        i32.add
        local.tee 1
        local.get 2
        i32.ge_u
        br_if 0 (;@1;)
        i32.const 0
        i32.const 0
        call $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec12handle_error
        unreachable
      end
      local.get 5
      i32.const 4
      i32.add
      local.get 0
      i32.load
      local.tee 2
      local.get 0
      i32.load offset=4
      local.get 1
      local.get 2
      i32.const 1
      i32.shl
      local.tee 2
      local.get 1
      local.get 2
      i32.gt_u
      select
      local.tee 2
      i32.const 8
      i32.const 4
      local.get 4
      i32.const 1
      i32.eq
      select
      local.tee 1
      local.get 2
      local.get 1
      i32.gt_u
      select
      local.tee 2
      local.get 3
      local.get 4
      call $_RNvMs4_NtCsewWLk9TkM7w_5alloc7raw_vecNtB5_11RawVecInner11finish_growCsdl5sGgnNXvY_3std
      block ;; label = @1
        local.get 5
        i32.load offset=4
        i32.const 1
        i32.ne
        br_if 0 (;@1;)
        local.get 5
        i32.load offset=8
        local.get 5
        i32.load offset=12
        call $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec12handle_error
        unreachable
      end
      local.get 5
      i32.load offset=8
      local.set 4
      local.get 0
      local.get 2
      i32.store
      local.get 0
      local.get 4
      i32.store offset=4
      local.get 5
      i32.const 16
      i32.add
      global.set $__stack_pointer
    )
    (func $_RINvNtNtCsdl5sGgnNXvY_3std3sys9backtrace26___rust_end_short_backtraceNCNvNtB6_5alloc8rust_oom0zEB6_ (;16;) (type 4) (param i32)
      local.get 0
      call $_RNCNvNtCsdl5sGgnNXvY_3std5alloc8rust_oom0B5_
      unreachable
    )
    (func $_RNCNvNtCsdl5sGgnNXvY_3std5alloc8rust_oom0B5_ (;17;) (type 4) (param i32)
      local.get 0
      i32.load
      local.get 0
      i32.load offset=4
      i32.const 0
      i32.load offset=1049256
      local.tee 0
      i32.const 3
      local.get 0
      select
      call_indirect (type 0)
      unreachable
    )
    (func $_RINvNtNtCsdl5sGgnNXvY_3std3sys9backtrace26___rust_end_short_backtraceNCNvNtB6_9panicking13panic_handler0zEB6_ (;18;) (type 4) (param i32)
      local.get 0
      call $_RNCNvNtCsdl5sGgnNXvY_3std9panicking13panic_handler0B5_
      unreachable
    )
    (func $_RNCNvNtCsdl5sGgnNXvY_3std9panicking13panic_handler0B5_ (;19;) (type 4) (param i32)
      (local i32 i32 i32)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 1
      global.set $__stack_pointer
      block ;; label = @1
        local.get 0
        i32.load
        local.tee 2
        i32.load offset=4
        local.tee 3
        i32.const 1
        i32.and
        i32.eqz
        br_if 0 (;@1;)
        local.get 2
        i32.load
        local.set 2
        local.get 1
        local.get 3
        i32.const 1
        i32.shr_u
        i32.store offset=4
        local.get 1
        local.get 2
        i32.store
        local.get 1
        i32.const 1048624
        local.get 0
        i32.load offset=4
        local.get 0
        i32.load offset=8
        local.tee 0
        i32.load8_u offset=8
        local.get 0
        i32.load8_u offset=9
        call $_RNvNtCsdl5sGgnNXvY_3std9panicking15panic_with_hook
        unreachable
      end
      local.get 1
      i32.const -1
      i32.store
      local.get 1
      local.get 0
      i32.store offset=12
      local.get 1
      i32.const 1048652
      local.get 0
      i32.load offset=4
      local.get 0
      i32.load offset=8
      local.tee 0
      i32.load8_u offset=8
      local.get 0
      i32.load8_u offset=9
      call $_RNvNtCsdl5sGgnNXvY_3std9panicking15panic_with_hook
      unreachable
    )
    (func $_RNvMs4_NtCsewWLk9TkM7w_5alloc7raw_vecNtB5_11RawVecInner11finish_growCsdl5sGgnNXvY_3std (;20;) (type 8) (param i32 i32 i32 i32 i32 i32)
      (local i32 i32 i64)
      i32.const 1
      local.set 6
      i32.const 4
      local.set 7
      block ;; label = @1
        block ;; label = @2
          local.get 5
          i64.extend_i32_u
          local.get 3
          i64.extend_i32_u
          i64.mul
          local.tee 8
          i64.const 32
          i64.shr_u
          i32.wrap_i64
          i32.eqz
          br_if 0 (;@2;)
          i32.const 0
          local.set 3
          br 1 (;@1;)
        end
        block ;; label = @2
          local.get 8
          i32.wrap_i64
          local.tee 3
          i32.const -2147483648
          local.get 4
          i32.sub
          i32.le_u
          br_if 0 (;@2;)
          i32.const 0
          local.set 3
          br 1 (;@1;)
        end
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                local.get 1
                i32.eqz
                br_if 0 (;@5;)
                local.get 2
                local.get 5
                local.get 1
                i32.mul
                local.get 4
                local.get 3
                call $_RNvCs9wFQrvczXsK_7___rustc14___rust_realloc
                local.set 7
                br 1 (;@4;)
              end
              block ;; label = @5
                local.get 3
                br_if 0 (;@5;)
                local.get 4
                local.set 7
                br 2 (;@3;)
              end
              call $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2
              local.get 3
              local.get 4
              call $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc
              local.set 7
            end
            local.get 7
            br_if 0 (;@3;)
            local.get 0
            local.get 4
            i32.store offset=4
            br 1 (;@2;)
          end
          local.get 0
          local.get 7
          i32.store offset=4
          i32.const 0
          local.set 6
        end
        i32.const 8
        local.set 7
      end
      local.get 0
      local.get 7
      i32.add
      local.get 3
      i32.store
      local.get 0
      local.get 6
      i32.store
    )
    (func $_RNvNtCsdl5sGgnNXvY_3std9panicking15panic_with_hook (;21;) (type 7) (param i32 i32 i32 i32 i32)
      (local i32 i32 i32)
      global.get $__stack_pointer
      i32.const 32
      i32.sub
      local.tee 5
      global.set $__stack_pointer
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    block ;; label = @8
                      i32.const 1
                      call $_RNvNtNtCsdl5sGgnNXvY_3std9panicking11panic_count8increase
                      i32.const 255
                      i32.and
                      br_table 4 (;@4;) 1 (;@7;) 0 (;@8;) 1 (;@7;)
                    end
                    i32.const 0
                    i32.load offset=1049260
                    local.tee 6
                    i32.const -1
                    i32.le_s
                    br_if 3 (;@4;)
                    local.get 6
                    i32.const 1
                    i32.add
                    local.tee 7
                    local.get 6
                    i32.lt_s
                    br_if 4 (;@3;)
                    i32.const 0
                    local.get 7
                    i32.store offset=1049260
                    i32.const 0
                    i32.load offset=1049264
                    i32.eqz
                    br_if 1 (;@6;)
                    local.get 5
                    i32.const 8
                    i32.add
                    local.get 0
                    local.get 1
                    i32.load offset=20
                    call_indirect (type 0)
                    local.get 5
                    local.get 4
                    i32.store8 offset=29
                    local.get 5
                    local.get 3
                    i32.store8 offset=28
                    local.get 5
                    local.get 2
                    i32.store offset=24
                    local.get 5
                    local.get 5
                    i64.load offset=8
                    i64.store offset=16 align=4
                    i32.const 0
                    i32.load offset=1049264
                    local.get 5
                    i32.const 16
                    i32.add
                    i32.const 0
                    i32.load offset=1049268
                    i32.load offset=20
                    call_indirect (type 0)
                    br 2 (;@5;)
                  end
                  local.get 5
                  local.get 0
                  local.get 1
                  i32.load offset=24
                  call_indirect (type 0)
                  unreachable
                end
                i32.const -1
                local.get 5
                call $_RINvNtCsdkdt1aaAg1T_4core3ptr9drop_glueINtNtB4_6option6OptionINtNtCsewWLk9TkM7w_5alloc3vec3VechEEECsdl5sGgnNXvY_3std
              end
              i32.const 0
              i32.const 0
              i32.load offset=1049260
              local.tee 5
              i32.const -1
              i32.add
              i32.store offset=1049260
              local.get 5
              i32.const 0
              i32.le_s
              br_if 2 (;@2;)
              i32.const 0
              i32.const 0
              i32.store8 offset=1049252
              local.get 3
              br_if 3 (;@1;)
            end
            unreachable
          end
          i32.const 1049060
          i32.const 28
          i32.const 1049088
          call $_RNvNtCsdkdt1aaAg1T_4core6option13expect_failed
          unreachable
        end
        i32.const 1049136
        i32.const 77
        i32.const 1049176
        call $_RNvNtCsdkdt1aaAg1T_4core9panicking9panic_fmt
        unreachable
      end
      local.get 0
      local.get 1
      call $_RNvCs9wFQrvczXsK_7___rustc10rust_panic
      unreachable
    )
    (func $_RNvNtCsdl5sGgnNXvY_3std5alloc24default_alloc_error_hook (;22;) (type 0) (param i32 i32)
      i32.const 0
      i32.const 1
      i32.store8 offset=1049728
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc10rust_panic (;23;) (type 0) (param i32 i32)
      local.get 0
      local.get 1
      call $_RNvCs9wFQrvczXsK_7___rustc18___rust_start_panic
      drop
      unreachable
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc11___rdl_alloc (;24;) (type 2) (param i32 i32) (result i32)
      block ;; label = @1
        local.get 1
        i32.const 9
        i32.lt_u
        br_if 0 (;@1;)
        local.get 1
        local.get 0
        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE8memalignCsdl5sGgnNXvY_3std
        return
      end
      local.get 0
      call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE6mallocCsdl5sGgnNXvY_3std
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE8memalignCsdl5sGgnNXvY_3std (;25;) (type 2) (param i32 i32) (result i32)
      (local i32 i32 i32 i32 i32)
      i32.const 0
      local.set 2
      block ;; label = @1
        local.get 1
        i32.const -65587
        local.get 0
        i32.const 16
        local.get 0
        i32.const 16
        i32.gt_u
        select
        local.tee 0
        i32.sub
        i32.ge_u
        br_if 0 (;@1;)
        local.get 0
        i32.const 16
        local.get 1
        i32.const 11
        i32.add
        i32.const -8
        i32.and
        local.get 1
        i32.const 11
        i32.lt_u
        select
        local.tee 3
        i32.add
        i32.const 12
        i32.add
        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE6mallocCsdl5sGgnNXvY_3std
        local.tee 1
        i32.eqz
        br_if 0 (;@1;)
        local.get 1
        i32.const -8
        i32.add
        local.set 2
        block ;; label = @2
          block ;; label = @3
            local.get 0
            i32.const -1
            i32.add
            local.tee 4
            local.get 1
            i32.and
            br_if 0 (;@3;)
            local.get 2
            local.set 0
            br 1 (;@2;)
          end
          local.get 1
          i32.const -4
          i32.add
          local.tee 5
          i32.load
          local.tee 6
          i32.const -8
          i32.and
          local.get 4
          local.get 1
          i32.add
          i32.const 0
          local.get 0
          i32.sub
          i32.and
          i32.const -8
          i32.add
          local.tee 1
          i32.const 0
          local.get 0
          local.get 1
          local.get 2
          i32.sub
          i32.const 16
          i32.gt_u
          select
          i32.add
          local.tee 0
          local.get 2
          i32.sub
          local.tee 1
          i32.sub
          local.set 4
          block ;; label = @3
            local.get 6
            i32.const 3
            i32.and
            i32.eqz
            br_if 0 (;@3;)
            local.get 0
            local.get 4
            local.get 0
            i32.load offset=4
            i32.const 1
            i32.and
            i32.or
            i32.const 2
            i32.or
            i32.store offset=4
            local.get 0
            local.get 4
            i32.add
            local.tee 4
            local.get 4
            i32.load offset=4
            i32.const 1
            i32.or
            i32.store offset=4
            local.get 5
            local.get 1
            local.get 5
            i32.load
            i32.const 1
            i32.and
            i32.or
            i32.const 2
            i32.or
            i32.store
            local.get 2
            local.get 1
            i32.add
            local.tee 4
            local.get 4
            i32.load offset=4
            i32.const 1
            i32.or
            i32.store offset=4
            local.get 2
            local.get 1
            call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE13dispose_chunkCsdl5sGgnNXvY_3std
            br 1 (;@2;)
          end
          local.get 2
          i32.load
          local.set 2
          local.get 0
          local.get 4
          i32.store offset=4
          local.get 0
          local.get 2
          local.get 1
          i32.add
          i32.store
        end
        block ;; label = @2
          local.get 0
          i32.load offset=4
          local.tee 1
          i32.const 3
          i32.and
          i32.eqz
          br_if 0 (;@2;)
          local.get 1
          i32.const -8
          i32.and
          local.tee 2
          local.get 3
          i32.const 16
          i32.add
          i32.le_u
          br_if 0 (;@2;)
          local.get 0
          local.get 3
          local.get 1
          i32.const 1
          i32.and
          i32.or
          i32.const 2
          i32.or
          i32.store offset=4
          local.get 0
          local.get 3
          i32.add
          local.tee 1
          local.get 2
          local.get 3
          i32.sub
          local.tee 3
          i32.const 3
          i32.or
          i32.store offset=4
          local.get 0
          local.get 2
          i32.add
          local.tee 2
          local.get 2
          i32.load offset=4
          i32.const 1
          i32.or
          i32.store offset=4
          local.get 1
          local.get 3
          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE13dispose_chunkCsdl5sGgnNXvY_3std
        end
        local.get 0
        i32.const 8
        i32.add
        local.set 2
      end
      local.get 2
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE6mallocCsdl5sGgnNXvY_3std (;26;) (type 9) (param i32) (result i32)
      (local i32 i32 i32 i32 i32 i32 i32 i32 i32 i64)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 1
      global.set $__stack_pointer
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              local.get 0
              i32.const 245
              i32.lt_u
              br_if 0 (;@4;)
              block ;; label = @5
                local.get 0
                i32.const -65588
                i32.le_u
                br_if 0 (;@5;)
                i32.const 0
                local.set 0
                br 4 (;@1;)
              end
              local.get 0
              i32.const 11
              i32.add
              local.tee 2
              i32.const -8
              i32.and
              local.set 3
              i32.const 0
              i32.load offset=1049688
              local.tee 4
              i32.eqz
              br_if 2 (;@2;)
              i32.const 31
              local.set 5
              local.get 0
              i32.const 16777205
              i32.ge_u
              br_if 1 (;@3;)
              local.get 3
              i32.const 38
              local.get 2
              i32.const 8
              i32.shr_u
              i32.clz
              local.tee 0
              i32.sub
              i32.shr_u
              i32.const 1
              i32.and
              local.get 0
              i32.const 1
              i32.shl
              i32.sub
              i32.const 62
              i32.add
              local.set 5
              br 1 (;@3;)
            end
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    block ;; label = @8
                      block ;; label = @9
                        i32.const 0
                        i32.load offset=1049684
                        local.tee 6
                        i32.const 16
                        local.get 0
                        i32.const 11
                        i32.add
                        i32.const 504
                        i32.and
                        local.get 0
                        i32.const 11
                        i32.lt_u
                        select
                        local.tee 3
                        i32.const 3
                        i32.shr_u
                        local.tee 2
                        i32.shr_u
                        local.tee 0
                        i32.const 3
                        i32.and
                        i32.eqz
                        br_if 0 (;@9;)
                        local.get 0
                        i32.const -1
                        i32.xor
                        i32.const 1
                        i32.and
                        local.get 2
                        i32.add
                        local.tee 7
                        i32.const 3
                        i32.shl
                        local.tee 3
                        i32.const 1049420
                        i32.add
                        local.tee 0
                        local.get 3
                        i32.const 1049428
                        i32.add
                        i32.load
                        local.tee 2
                        i32.load offset=8
                        local.tee 8
                        i32.eq
                        br_if 1 (;@8;)
                        local.get 8
                        local.get 0
                        i32.store offset=12
                        local.get 0
                        local.get 8
                        i32.store offset=8
                        br 2 (;@7;)
                      end
                      local.get 3
                      i32.const 0
                      i32.load offset=1049692
                      i32.le_u
                      br_if 6 (;@2;)
                      local.get 0
                      br_if 2 (;@6;)
                      i32.const 0
                      i32.load offset=1049688
                      local.tee 0
                      i32.eqz
                      br_if 6 (;@2;)
                      local.get 0
                      i32.ctz
                      i32.const 2
                      i32.shl
                      i32.const 1049276
                      i32.add
                      i32.load
                      local.tee 8
                      i32.load offset=4
                      i32.const -8
                      i32.and
                      local.get 3
                      i32.sub
                      local.set 2
                      local.get 8
                      local.set 6
                      loop ;; label = @9
                        block ;; label = @10
                          local.get 8
                          i32.load offset=16
                          local.tee 0
                          br_if 0 (;@10;)
                          local.get 8
                          i32.load offset=20
                          local.tee 0
                          br_if 0 (;@10;)
                          local.get 6
                          i32.load offset=24
                          local.set 5
                          block ;; label = @11
                            block ;; label = @12
                              block ;; label = @13
                                local.get 6
                                i32.load offset=12
                                local.tee 0
                                local.get 6
                                i32.ne
                                br_if 0 (;@13;)
                                local.get 6
                                i32.const 20
                                i32.const 16
                                local.get 6
                                i32.load offset=20
                                local.tee 0
                                select
                                i32.add
                                i32.load
                                local.tee 8
                                br_if 1 (;@12;)
                                i32.const 0
                                local.set 0
                                br 2 (;@11;)
                              end
                              local.get 6
                              i32.load offset=8
                              local.tee 8
                              local.get 0
                              i32.store offset=12
                              local.get 0
                              local.get 8
                              i32.store offset=8
                              br 1 (;@11;)
                            end
                            local.get 6
                            i32.const 20
                            i32.add
                            local.get 6
                            i32.const 16
                            i32.add
                            local.get 0
                            select
                            local.set 7
                            loop ;; label = @12
                              local.get 7
                              local.set 9
                              local.get 8
                              local.tee 0
                              i32.const 20
                              i32.add
                              local.get 0
                              i32.const 16
                              i32.add
                              local.get 0
                              i32.load offset=20
                              local.tee 8
                              select
                              local.set 7
                              local.get 0
                              i32.const 20
                              i32.const 16
                              local.get 8
                              select
                              i32.add
                              i32.load
                              local.tee 8
                              br_if 0 (;@12;)
                            end
                            local.get 9
                            i32.const 0
                            i32.store
                          end
                          local.get 5
                          i32.eqz
                          br_if 6 (;@4;)
                          block ;; label = @11
                            block ;; label = @12
                              local.get 6
                              local.get 6
                              i32.load offset=28
                              i32.const 2
                              i32.shl
                              i32.const 1049276
                              i32.add
                              local.tee 8
                              i32.load
                              i32.eq
                              br_if 0 (;@12;)
                              block ;; label = @13
                                local.get 5
                                i32.load offset=16
                                local.get 6
                                i32.eq
                                br_if 0 (;@13;)
                                local.get 5
                                local.get 0
                                i32.store offset=20
                                local.get 0
                                br_if 2 (;@11;)
                                br 9 (;@4;)
                              end
                              local.get 5
                              local.get 0
                              i32.store offset=16
                              local.get 0
                              br_if 1 (;@11;)
                              br 8 (;@4;)
                            end
                            local.get 8
                            local.get 0
                            i32.store
                            local.get 0
                            i32.eqz
                            br_if 6 (;@5;)
                          end
                          local.get 0
                          local.get 5
                          i32.store offset=24
                          block ;; label = @11
                            local.get 6
                            i32.load offset=16
                            local.tee 8
                            i32.eqz
                            br_if 0 (;@11;)
                            local.get 0
                            local.get 8
                            i32.store offset=16
                            local.get 8
                            local.get 0
                            i32.store offset=24
                          end
                          local.get 6
                          i32.load offset=20
                          local.tee 8
                          i32.eqz
                          br_if 6 (;@4;)
                          local.get 0
                          local.get 8
                          i32.store offset=20
                          local.get 8
                          local.get 0
                          i32.store offset=24
                          br 6 (;@4;)
                        end
                        local.get 0
                        i32.load offset=4
                        i32.const -8
                        i32.and
                        local.get 3
                        i32.sub
                        local.tee 8
                        local.get 2
                        local.get 8
                        local.get 2
                        i32.lt_u
                        local.tee 8
                        select
                        local.set 2
                        local.get 0
                        local.get 6
                        local.get 8
                        select
                        local.set 6
                        local.get 0
                        local.set 8
                        br 0 (;@9;)
                      end
                    end
                    i32.const 0
                    local.get 6
                    i32.const -2
                    local.get 7
                    i32.rotl
                    i32.and
                    i32.store offset=1049684
                  end
                  local.get 2
                  i32.const 8
                  i32.add
                  local.set 0
                  local.get 2
                  local.get 3
                  i32.const 3
                  i32.or
                  i32.store offset=4
                  local.get 2
                  local.get 3
                  i32.add
                  local.tee 3
                  local.get 3
                  i32.load offset=4
                  i32.const 1
                  i32.or
                  i32.store offset=4
                  br 5 (;@1;)
                end
                block ;; label = @6
                  block ;; label = @7
                    local.get 0
                    local.get 2
                    i32.shl
                    i32.const 2
                    local.get 2
                    i32.shl
                    local.tee 0
                    i32.const 0
                    local.get 0
                    i32.sub
                    i32.or
                    i32.and
                    i32.ctz
                    local.tee 9
                    i32.const 3
                    i32.shl
                    local.tee 2
                    i32.const 1049420
                    i32.add
                    local.tee 8
                    local.get 2
                    i32.const 1049428
                    i32.add
                    i32.load
                    local.tee 0
                    i32.load offset=8
                    local.tee 7
                    i32.eq
                    br_if 0 (;@7;)
                    local.get 7
                    local.get 8
                    i32.store offset=12
                    local.get 8
                    local.get 7
                    i32.store offset=8
                    br 1 (;@6;)
                  end
                  i32.const 0
                  local.get 6
                  i32.const -2
                  local.get 9
                  i32.rotl
                  i32.and
                  i32.store offset=1049684
                end
                local.get 0
                local.get 3
                i32.const 3
                i32.or
                i32.store offset=4
                local.get 0
                local.get 3
                i32.add
                local.tee 6
                local.get 2
                local.get 3
                i32.sub
                local.tee 8
                i32.const 1
                i32.or
                i32.store offset=4
                local.get 0
                local.get 2
                i32.add
                local.get 8
                i32.store
                block ;; label = @6
                  i32.const 0
                  i32.load offset=1049692
                  local.tee 2
                  i32.eqz
                  br_if 0 (;@6;)
                  i32.const 0
                  i32.load offset=1049700
                  local.set 3
                  block ;; label = @7
                    block ;; label = @8
                      i32.const 0
                      i32.load offset=1049684
                      local.tee 7
                      i32.const 1
                      local.get 2
                      i32.const 3
                      i32.shr_u
                      i32.shl
                      local.tee 9
                      i32.and
                      br_if 0 (;@8;)
                      i32.const 0
                      local.get 7
                      local.get 9
                      i32.or
                      i32.store offset=1049684
                      local.get 2
                      i32.const -8
                      i32.and
                      i32.const 1049420
                      i32.add
                      local.tee 2
                      local.set 7
                      br 1 (;@7;)
                    end
                    local.get 2
                    i32.const -8
                    i32.and
                    local.tee 2
                    i32.const 1049420
                    i32.add
                    local.set 7
                    local.get 2
                    i32.const 1049428
                    i32.add
                    i32.load
                    local.set 2
                  end
                  local.get 7
                  local.get 3
                  i32.store offset=8
                  local.get 2
                  local.get 3
                  i32.store offset=12
                  local.get 3
                  local.get 7
                  i32.store offset=12
                  local.get 3
                  local.get 2
                  i32.store offset=8
                end
                local.get 0
                i32.const 8
                i32.add
                local.set 0
                i32.const 0
                local.get 6
                i32.store offset=1049700
                i32.const 0
                local.get 8
                i32.store offset=1049692
                br 4 (;@1;)
              end
              i32.const 0
              i32.const 0
              i32.load offset=1049688
              i32.const -2
              local.get 6
              i32.load offset=28
              i32.rotl
              i32.and
              i32.store offset=1049688
            end
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  local.get 2
                  i32.const 16
                  i32.lt_u
                  br_if 0 (;@6;)
                  local.get 6
                  local.get 3
                  i32.const 3
                  i32.or
                  i32.store offset=4
                  local.get 6
                  local.get 3
                  i32.add
                  local.tee 8
                  local.get 2
                  i32.const 1
                  i32.or
                  i32.store offset=4
                  local.get 8
                  local.get 2
                  i32.add
                  local.get 2
                  i32.store
                  i32.const 0
                  i32.load offset=1049692
                  local.tee 7
                  i32.eqz
                  br_if 1 (;@5;)
                  i32.const 0
                  i32.load offset=1049700
                  local.set 0
                  block ;; label = @7
                    block ;; label = @8
                      i32.const 0
                      i32.load offset=1049684
                      local.tee 9
                      i32.const 1
                      local.get 7
                      i32.const 3
                      i32.shr_u
                      i32.shl
                      local.tee 5
                      i32.and
                      br_if 0 (;@8;)
                      i32.const 0
                      local.get 9
                      local.get 5
                      i32.or
                      i32.store offset=1049684
                      local.get 7
                      i32.const -8
                      i32.and
                      i32.const 1049420
                      i32.add
                      local.tee 7
                      local.set 9
                      br 1 (;@7;)
                    end
                    local.get 7
                    i32.const -8
                    i32.and
                    local.tee 7
                    i32.const 1049420
                    i32.add
                    local.set 9
                    local.get 7
                    i32.const 1049428
                    i32.add
                    i32.load
                    local.set 7
                  end
                  local.get 9
                  local.get 0
                  i32.store offset=8
                  local.get 7
                  local.get 0
                  i32.store offset=12
                  local.get 0
                  local.get 9
                  i32.store offset=12
                  local.get 0
                  local.get 7
                  i32.store offset=8
                  br 1 (;@5;)
                end
                local.get 6
                local.get 2
                local.get 3
                i32.add
                local.tee 0
                i32.const 3
                i32.or
                i32.store offset=4
                local.get 6
                local.get 0
                i32.add
                local.tee 0
                local.get 0
                i32.load offset=4
                i32.const 1
                i32.or
                i32.store offset=4
                br 1 (;@4;)
              end
              i32.const 0
              local.get 8
              i32.store offset=1049700
              i32.const 0
              local.get 2
              i32.store offset=1049692
            end
            local.get 6
            i32.const 8
            i32.add
            local.tee 0
            i32.eqz
            br_if 1 (;@2;)
            br 2 (;@1;)
          end
          i32.const 0
          local.get 3
          i32.sub
          local.set 2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  local.get 5
                  i32.const 2
                  i32.shl
                  i32.const 1049276
                  i32.add
                  i32.load
                  local.tee 6
                  br_if 0 (;@6;)
                  i32.const 0
                  local.set 8
                  i32.const 0
                  local.set 0
                  br 1 (;@5;)
                end
                i32.const 0
                local.set 8
                local.get 3
                i32.const 0
                i32.const 25
                local.get 5
                i32.const 1
                i32.shr_u
                i32.sub
                local.get 5
                i32.const 31
                i32.eq
                select
                i32.shl
                local.set 7
                i32.const 0
                local.set 0
                loop ;; label = @6
                  block ;; label = @7
                    local.get 6
                    local.tee 6
                    i32.load offset=4
                    i32.const -8
                    i32.and
                    local.tee 9
                    local.get 3
                    i32.lt_u
                    br_if 0 (;@7;)
                    local.get 9
                    local.get 3
                    i32.sub
                    local.tee 9
                    local.get 2
                    i32.ge_u
                    br_if 0 (;@7;)
                    local.get 6
                    local.set 8
                    local.get 9
                    local.set 2
                    local.get 9
                    br_if 0 (;@7;)
                    i32.const 0
                    local.set 2
                    local.get 6
                    local.set 0
                    local.get 6
                    local.set 8
                    br 3 (;@4;)
                  end
                  local.get 6
                  i32.load offset=20
                  local.tee 9
                  local.get 0
                  local.get 9
                  local.get 6
                  local.get 7
                  i32.const 29
                  i32.shr_u
                  i32.const 4
                  i32.and
                  i32.add
                  i32.load offset=16
                  local.tee 6
                  i32.ne
                  select
                  local.get 0
                  local.get 9
                  select
                  local.set 0
                  local.get 7
                  i32.const 1
                  i32.shl
                  local.set 7
                  local.get 6
                  br_if 0 (;@6;)
                end
              end
              block ;; label = @5
                local.get 0
                local.get 8
                i32.or
                br_if 0 (;@5;)
                i32.const 0
                local.set 8
                i32.const 2
                local.get 5
                i32.shl
                local.tee 0
                i32.const 0
                local.get 0
                i32.sub
                i32.or
                local.get 4
                i32.and
                local.tee 0
                i32.eqz
                br_if 3 (;@2;)
                local.get 0
                i32.ctz
                i32.const 2
                i32.shl
                i32.const 1049276
                i32.add
                i32.load
                local.set 0
              end
              local.get 0
              i32.eqz
              br_if 1 (;@3;)
            end
            loop ;; label = @4
              local.get 0
              i32.load offset=4
              i32.const -8
              i32.and
              local.tee 6
              local.get 3
              i32.sub
              local.tee 7
              local.get 2
              local.get 7
              local.get 2
              i32.lt_u
              local.tee 9
              select
              local.set 5
              local.get 6
              local.get 3
              i32.lt_u
              local.set 7
              local.get 0
              local.get 8
              local.get 9
              select
              local.set 9
              block ;; label = @5
                local.get 0
                i32.load offset=16
                local.tee 6
                br_if 0 (;@5;)
                local.get 0
                i32.load offset=20
                local.set 6
              end
              local.get 2
              local.get 5
              local.get 7
              select
              local.set 2
              local.get 8
              local.get 9
              local.get 7
              select
              local.set 8
              local.get 6
              local.set 0
              local.get 6
              br_if 0 (;@4;)
            end
          end
          local.get 8
          i32.eqz
          br_if 0 (;@2;)
          block ;; label = @3
            i32.const 0
            i32.load offset=1049692
            local.tee 0
            local.get 3
            i32.lt_u
            br_if 0 (;@3;)
            local.get 2
            local.get 0
            local.get 3
            i32.sub
            i32.ge_u
            br_if 1 (;@2;)
          end
          local.get 8
          i32.load offset=24
          local.set 5
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                local.get 8
                i32.load offset=12
                local.tee 0
                local.get 8
                i32.ne
                br_if 0 (;@5;)
                local.get 8
                i32.const 20
                i32.const 16
                local.get 8
                i32.load offset=20
                local.tee 0
                select
                i32.add
                i32.load
                local.tee 6
                br_if 1 (;@4;)
                i32.const 0
                local.set 0
                br 2 (;@3;)
              end
              local.get 8
              i32.load offset=8
              local.tee 6
              local.get 0
              i32.store offset=12
              local.get 0
              local.get 6
              i32.store offset=8
              br 1 (;@3;)
            end
            local.get 8
            i32.const 20
            i32.add
            local.get 8
            i32.const 16
            i32.add
            local.get 0
            select
            local.set 7
            loop ;; label = @4
              local.get 7
              local.set 9
              local.get 6
              local.tee 0
              i32.const 20
              i32.add
              local.get 0
              i32.const 16
              i32.add
              local.get 0
              i32.load offset=20
              local.tee 6
              select
              local.set 7
              local.get 0
              i32.const 20
              i32.const 16
              local.get 6
              select
              i32.add
              i32.load
              local.tee 6
              br_if 0 (;@4;)
            end
            local.get 9
            i32.const 0
            i32.store
          end
          block ;; label = @3
            local.get 5
            i32.eqz
            br_if 0 (;@3;)
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  local.get 8
                  local.get 8
                  i32.load offset=28
                  i32.const 2
                  i32.shl
                  i32.const 1049276
                  i32.add
                  local.tee 6
                  i32.load
                  i32.eq
                  br_if 0 (;@6;)
                  block ;; label = @7
                    local.get 5
                    i32.load offset=16
                    local.get 8
                    i32.eq
                    br_if 0 (;@7;)
                    local.get 5
                    local.get 0
                    i32.store offset=20
                    local.get 0
                    br_if 2 (;@5;)
                    br 4 (;@3;)
                  end
                  local.get 5
                  local.get 0
                  i32.store offset=16
                  local.get 0
                  br_if 1 (;@5;)
                  br 3 (;@3;)
                end
                local.get 6
                local.get 0
                i32.store
                local.get 0
                i32.eqz
                br_if 1 (;@4;)
              end
              local.get 0
              local.get 5
              i32.store offset=24
              block ;; label = @5
                local.get 8
                i32.load offset=16
                local.tee 6
                i32.eqz
                br_if 0 (;@5;)
                local.get 0
                local.get 6
                i32.store offset=16
                local.get 6
                local.get 0
                i32.store offset=24
              end
              local.get 8
              i32.load offset=20
              local.tee 6
              i32.eqz
              br_if 1 (;@3;)
              local.get 0
              local.get 6
              i32.store offset=20
              local.get 6
              local.get 0
              i32.store offset=24
              br 1 (;@3;)
            end
            i32.const 0
            i32.const 0
            i32.load offset=1049688
            i32.const -2
            local.get 8
            i32.load offset=28
            i32.rotl
            i32.and
            i32.store offset=1049688
          end
          block ;; label = @3
            block ;; label = @4
              local.get 2
              i32.const 16
              i32.lt_u
              br_if 0 (;@4;)
              local.get 8
              local.get 3
              i32.const 3
              i32.or
              i32.store offset=4
              local.get 8
              local.get 3
              i32.add
              local.tee 0
              local.get 2
              i32.const 1
              i32.or
              i32.store offset=4
              local.get 0
              local.get 2
              i32.add
              local.get 2
              i32.store
              block ;; label = @5
                local.get 2
                i32.const 256
                i32.lt_u
                br_if 0 (;@5;)
                local.get 0
                local.get 2
                call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std
                br 2 (;@3;)
              end
              block ;; label = @5
                block ;; label = @6
                  i32.const 0
                  i32.load offset=1049684
                  local.tee 6
                  i32.const 1
                  local.get 2
                  i32.const 3
                  i32.shr_u
                  i32.shl
                  local.tee 7
                  i32.and
                  br_if 0 (;@6;)
                  i32.const 0
                  local.get 6
                  local.get 7
                  i32.or
                  i32.store offset=1049684
                  local.get 2
                  i32.const 248
                  i32.and
                  i32.const 1049420
                  i32.add
                  local.tee 2
                  local.set 6
                  br 1 (;@5;)
                end
                local.get 2
                i32.const 248
                i32.and
                local.tee 2
                i32.const 1049420
                i32.add
                local.set 6
                local.get 2
                i32.const 1049428
                i32.add
                i32.load
                local.set 2
              end
              local.get 6
              local.get 0
              i32.store offset=8
              local.get 2
              local.get 0
              i32.store offset=12
              local.get 0
              local.get 6
              i32.store offset=12
              local.get 0
              local.get 2
              i32.store offset=8
              br 1 (;@3;)
            end
            local.get 8
            local.get 2
            local.get 3
            i32.add
            local.tee 0
            i32.const 3
            i32.or
            i32.store offset=4
            local.get 8
            local.get 0
            i32.add
            local.tee 0
            local.get 0
            i32.load offset=4
            i32.const 1
            i32.or
            i32.store offset=4
          end
          local.get 8
          i32.const 8
          i32.add
          local.tee 0
          br_if 1 (;@1;)
        end
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    i32.const 0
                    i32.load offset=1049692
                    local.tee 0
                    local.get 3
                    i32.ge_u
                    br_if 0 (;@7;)
                    block ;; label = @8
                      i32.const 0
                      i32.load offset=1049696
                      local.tee 0
                      local.get 3
                      i32.gt_u
                      br_if 0 (;@8;)
                      local.get 1
                      i32.const 4
                      i32.add
                      i32.const 1049728
                      local.get 3
                      i32.const 65583
                      i32.add
                      i32.const -65536
                      i32.and
                      call $_RNvXs_NtCsgis5MWNmFLl_8dlmalloc3sysNtB4_6SystemNtB6_9Allocator5alloc
                      block ;; label = @9
                        local.get 1
                        i32.load offset=4
                        local.tee 6
                        br_if 0 (;@9;)
                        i32.const 0
                        local.set 0
                        br 8 (;@1;)
                      end
                      local.get 1
                      i32.load offset=12
                      local.set 5
                      i32.const 0
                      i32.const 0
                      i32.load offset=1049708
                      local.get 1
                      i32.load offset=8
                      local.tee 9
                      i32.add
                      local.tee 0
                      i32.store offset=1049708
                      i32.const 0
                      local.get 0
                      i32.const 0
                      i32.load offset=1049712
                      local.tee 2
                      local.get 0
                      local.get 2
                      i32.gt_u
                      select
                      i32.store offset=1049712
                      block ;; label = @9
                        block ;; label = @10
                          block ;; label = @11
                            i32.const 0
                            i32.load offset=1049704
                            local.tee 2
                            i32.eqz
                            br_if 0 (;@11;)
                            i32.const 1049404
                            local.set 0
                            loop ;; label = @12
                              local.get 6
                              local.get 0
                              i32.load
                              local.tee 8
                              local.get 0
                              i32.load offset=4
                              local.tee 7
                              i32.add
                              i32.eq
                              br_if 2 (;@10;)
                              local.get 0
                              i32.load offset=8
                              local.tee 0
                              br_if 0 (;@12;)
                              br 3 (;@9;)
                            end
                          end
                          block ;; label = @11
                            block ;; label = @12
                              i32.const 0
                              i32.load offset=1049720
                              local.tee 0
                              i32.eqz
                              br_if 0 (;@12;)
                              local.get 6
                              local.get 0
                              i32.ge_u
                              br_if 1 (;@11;)
                            end
                            i32.const 0
                            local.get 6
                            i32.store offset=1049720
                          end
                          i32.const 0
                          i32.const 4095
                          i32.store offset=1049724
                          i32.const 0
                          local.get 5
                          i32.store offset=1049416
                          i32.const 0
                          local.get 9
                          i32.store offset=1049408
                          i32.const 0
                          local.get 6
                          i32.store offset=1049404
                          i32.const 0
                          i32.const 1049420
                          i32.store offset=1049432
                          i32.const 0
                          i32.const 1049428
                          i32.store offset=1049440
                          i32.const 0
                          i32.const 1049420
                          i32.store offset=1049428
                          i32.const 0
                          i32.const 1049436
                          i32.store offset=1049448
                          i32.const 0
                          i32.const 1049428
                          i32.store offset=1049436
                          i32.const 0
                          i32.const 1049444
                          i32.store offset=1049456
                          i32.const 0
                          i32.const 1049436
                          i32.store offset=1049444
                          i32.const 0
                          i32.const 1049452
                          i32.store offset=1049464
                          i32.const 0
                          i32.const 1049444
                          i32.store offset=1049452
                          i32.const 0
                          i32.const 1049460
                          i32.store offset=1049472
                          i32.const 0
                          i32.const 1049452
                          i32.store offset=1049460
                          i32.const 0
                          i32.const 1049468
                          i32.store offset=1049480
                          i32.const 0
                          i32.const 1049460
                          i32.store offset=1049468
                          i32.const 0
                          i32.const 1049476
                          i32.store offset=1049488
                          i32.const 0
                          i32.const 1049468
                          i32.store offset=1049476
                          i32.const 0
                          i32.const 1049484
                          i32.store offset=1049496
                          i32.const 0
                          i32.const 1049476
                          i32.store offset=1049484
                          i32.const 0
                          i32.const 1049484
                          i32.store offset=1049492
                          i32.const 0
                          i32.const 1049492
                          i32.store offset=1049504
                          i32.const 0
                          i32.const 1049492
                          i32.store offset=1049500
                          i32.const 0
                          i32.const 1049500
                          i32.store offset=1049512
                          i32.const 0
                          i32.const 1049500
                          i32.store offset=1049508
                          i32.const 0
                          i32.const 1049508
                          i32.store offset=1049520
                          i32.const 0
                          i32.const 1049508
                          i32.store offset=1049516
                          i32.const 0
                          i32.const 1049516
                          i32.store offset=1049528
                          i32.const 0
                          i32.const 1049516
                          i32.store offset=1049524
                          i32.const 0
                          i32.const 1049524
                          i32.store offset=1049536
                          i32.const 0
                          i32.const 1049524
                          i32.store offset=1049532
                          i32.const 0
                          i32.const 1049532
                          i32.store offset=1049544
                          i32.const 0
                          i32.const 1049532
                          i32.store offset=1049540
                          i32.const 0
                          i32.const 1049540
                          i32.store offset=1049552
                          i32.const 0
                          i32.const 1049540
                          i32.store offset=1049548
                          i32.const 0
                          i32.const 1049548
                          i32.store offset=1049560
                          i32.const 0
                          i32.const 1049556
                          i32.store offset=1049568
                          i32.const 0
                          i32.const 1049548
                          i32.store offset=1049556
                          i32.const 0
                          i32.const 1049564
                          i32.store offset=1049576
                          i32.const 0
                          i32.const 1049556
                          i32.store offset=1049564
                          i32.const 0
                          i32.const 1049572
                          i32.store offset=1049584
                          i32.const 0
                          i32.const 1049564
                          i32.store offset=1049572
                          i32.const 0
                          i32.const 1049580
                          i32.store offset=1049592
                          i32.const 0
                          i32.const 1049572
                          i32.store offset=1049580
                          i32.const 0
                          i32.const 1049588
                          i32.store offset=1049600
                          i32.const 0
                          i32.const 1049580
                          i32.store offset=1049588
                          i32.const 0
                          i32.const 1049596
                          i32.store offset=1049608
                          i32.const 0
                          i32.const 1049588
                          i32.store offset=1049596
                          i32.const 0
                          i32.const 1049604
                          i32.store offset=1049616
                          i32.const 0
                          i32.const 1049596
                          i32.store offset=1049604
                          i32.const 0
                          i32.const 1049612
                          i32.store offset=1049624
                          i32.const 0
                          i32.const 1049604
                          i32.store offset=1049612
                          i32.const 0
                          i32.const 1049620
                          i32.store offset=1049632
                          i32.const 0
                          i32.const 1049612
                          i32.store offset=1049620
                          i32.const 0
                          i32.const 1049628
                          i32.store offset=1049640
                          i32.const 0
                          i32.const 1049620
                          i32.store offset=1049628
                          i32.const 0
                          i32.const 1049636
                          i32.store offset=1049648
                          i32.const 0
                          i32.const 1049628
                          i32.store offset=1049636
                          i32.const 0
                          i32.const 1049644
                          i32.store offset=1049656
                          i32.const 0
                          i32.const 1049636
                          i32.store offset=1049644
                          i32.const 0
                          i32.const 1049652
                          i32.store offset=1049664
                          i32.const 0
                          i32.const 1049644
                          i32.store offset=1049652
                          i32.const 0
                          i32.const 1049660
                          i32.store offset=1049672
                          i32.const 0
                          i32.const 1049652
                          i32.store offset=1049660
                          i32.const 0
                          i32.const 1049668
                          i32.store offset=1049680
                          i32.const 0
                          i32.const 1049660
                          i32.store offset=1049668
                          i32.const 0
                          local.get 6
                          i32.const 15
                          i32.add
                          i32.const -8
                          i32.and
                          local.tee 0
                          i32.const -8
                          i32.add
                          local.tee 2
                          i32.store offset=1049704
                          i32.const 0
                          i32.const 1049668
                          i32.store offset=1049676
                          i32.const 0
                          local.get 6
                          local.get 0
                          i32.sub
                          local.get 9
                          i32.const -40
                          i32.add
                          local.tee 0
                          i32.add
                          i32.const 8
                          i32.add
                          local.tee 8
                          i32.store offset=1049696
                          local.get 2
                          local.get 8
                          i32.const 1
                          i32.or
                          i32.store offset=4
                          local.get 6
                          local.get 0
                          i32.add
                          i32.const 40
                          i32.store offset=4
                          i32.const 0
                          i32.const 2097152
                          i32.store offset=1049716
                          br 8 (;@2;)
                        end
                        local.get 2
                        local.get 6
                        i32.ge_u
                        br_if 0 (;@9;)
                        local.get 8
                        local.get 2
                        i32.gt_u
                        br_if 0 (;@9;)
                        local.get 0
                        i32.load offset=12
                        local.tee 8
                        i32.const 1
                        i32.and
                        br_if 0 (;@9;)
                        local.get 8
                        i32.const 1
                        i32.shr_u
                        local.get 5
                        i32.eq
                        br_if 3 (;@6;)
                      end
                      i32.const 0
                      i32.const 0
                      i32.load offset=1049720
                      local.tee 0
                      local.get 6
                      local.get 0
                      local.get 6
                      i32.lt_u
                      select
                      i32.store offset=1049720
                      local.get 6
                      local.get 9
                      i32.add
                      local.set 8
                      i32.const 1049404
                      local.set 0
                      block ;; label = @9
                        block ;; label = @10
                          block ;; label = @11
                            loop ;; label = @12
                              local.get 0
                              i32.load
                              local.tee 7
                              local.get 8
                              i32.eq
                              br_if 1 (;@11;)
                              local.get 0
                              i32.load offset=8
                              local.tee 0
                              br_if 0 (;@12;)
                              br 2 (;@10;)
                            end
                          end
                          local.get 0
                          i32.load offset=12
                          local.tee 8
                          i32.const 1
                          i32.and
                          br_if 0 (;@10;)
                          local.get 8
                          i32.const 1
                          i32.shr_u
                          local.get 5
                          i32.eq
                          br_if 1 (;@9;)
                        end
                        i32.const 1049404
                        local.set 0
                        block ;; label = @10
                          loop ;; label = @11
                            block ;; label = @12
                              local.get 0
                              i32.load
                              local.tee 8
                              local.get 2
                              i32.gt_u
                              br_if 0 (;@12;)
                              local.get 2
                              local.get 8
                              local.get 0
                              i32.load offset=4
                              i32.add
                              local.tee 8
                              i32.lt_u
                              br_if 2 (;@10;)
                            end
                            local.get 0
                            i32.load offset=8
                            local.set 0
                            br 0 (;@11;)
                          end
                        end
                        i32.const 0
                        local.get 6
                        i32.const 15
                        i32.add
                        i32.const -8
                        i32.and
                        local.tee 0
                        i32.const -8
                        i32.add
                        local.tee 7
                        i32.store offset=1049704
                        i32.const 0
                        local.get 6
                        local.get 0
                        i32.sub
                        local.get 9
                        i32.const -40
                        i32.add
                        local.tee 0
                        i32.add
                        i32.const 8
                        i32.add
                        local.tee 4
                        i32.store offset=1049696
                        local.get 7
                        local.get 4
                        i32.const 1
                        i32.or
                        i32.store offset=4
                        local.get 6
                        local.get 0
                        i32.add
                        i32.const 40
                        i32.store offset=4
                        i32.const 0
                        i32.const 2097152
                        i32.store offset=1049716
                        local.get 2
                        local.get 8
                        i32.const -32
                        i32.add
                        i32.const -8
                        i32.and
                        i32.const -8
                        i32.add
                        local.tee 0
                        local.get 0
                        local.get 2
                        i32.const 16
                        i32.add
                        i32.lt_u
                        select
                        local.tee 7
                        i32.const 27
                        i32.store offset=4
                        i32.const 0
                        i64.load offset=1049404 align=4
                        local.set 10
                        local.get 7
                        i32.const 16
                        i32.add
                        i32.const 0
                        i64.load offset=1049412 align=4
                        i64.store align=4
                        local.get 7
                        i32.const 8
                        i32.add
                        local.tee 0
                        local.get 10
                        i64.store align=4
                        i32.const 0
                        local.get 5
                        i32.store offset=1049416
                        i32.const 0
                        local.get 9
                        i32.store offset=1049408
                        i32.const 0
                        local.get 6
                        i32.store offset=1049404
                        i32.const 0
                        local.get 0
                        i32.store offset=1049412
                        local.get 7
                        i32.const 28
                        i32.add
                        local.set 0
                        loop ;; label = @10
                          local.get 0
                          i32.const 7
                          i32.store
                          local.get 0
                          i32.const 4
                          i32.add
                          local.tee 0
                          local.get 8
                          i32.lt_u
                          br_if 0 (;@10;)
                        end
                        local.get 7
                        local.get 2
                        i32.eq
                        br_if 7 (;@2;)
                        local.get 7
                        local.get 7
                        i32.load offset=4
                        i32.const -2
                        i32.and
                        i32.store offset=4
                        local.get 2
                        local.get 7
                        local.get 2
                        i32.sub
                        local.tee 0
                        i32.const 1
                        i32.or
                        i32.store offset=4
                        local.get 7
                        local.get 0
                        i32.store
                        block ;; label = @10
                          local.get 0
                          i32.const 256
                          i32.lt_u
                          br_if 0 (;@10;)
                          local.get 2
                          local.get 0
                          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std
                          br 8 (;@2;)
                        end
                        block ;; label = @10
                          block ;; label = @11
                            i32.const 0
                            i32.load offset=1049684
                            local.tee 8
                            i32.const 1
                            local.get 0
                            i32.const 3
                            i32.shr_u
                            i32.shl
                            local.tee 6
                            i32.and
                            br_if 0 (;@11;)
                            i32.const 0
                            local.get 8
                            local.get 6
                            i32.or
                            i32.store offset=1049684
                            local.get 0
                            i32.const 248
                            i32.and
                            i32.const 1049420
                            i32.add
                            local.tee 0
                            local.set 8
                            br 1 (;@10;)
                          end
                          local.get 0
                          i32.const 248
                          i32.and
                          local.tee 0
                          i32.const 1049420
                          i32.add
                          local.set 8
                          local.get 0
                          i32.const 1049428
                          i32.add
                          i32.load
                          local.set 0
                        end
                        local.get 8
                        local.get 2
                        i32.store offset=8
                        local.get 0
                        local.get 2
                        i32.store offset=12
                        local.get 2
                        local.get 8
                        i32.store offset=12
                        local.get 2
                        local.get 0
                        i32.store offset=8
                        br 7 (;@2;)
                      end
                      local.get 0
                      local.get 6
                      i32.store
                      local.get 0
                      local.get 0
                      i32.load offset=4
                      local.get 9
                      i32.add
                      i32.store offset=4
                      local.get 6
                      i32.const 15
                      i32.add
                      i32.const -8
                      i32.and
                      i32.const -8
                      i32.add
                      local.tee 8
                      local.get 3
                      i32.const 3
                      i32.or
                      i32.store offset=4
                      local.get 7
                      i32.const 15
                      i32.add
                      i32.const -8
                      i32.and
                      i32.const -8
                      i32.add
                      local.tee 2
                      local.get 8
                      local.get 3
                      i32.add
                      local.tee 0
                      i32.sub
                      local.set 3
                      local.get 2
                      i32.const 0
                      i32.load offset=1049704
                      i32.eq
                      br_if 3 (;@5;)
                      local.get 2
                      i32.const 0
                      i32.load offset=1049700
                      i32.eq
                      br_if 4 (;@4;)
                      block ;; label = @9
                        local.get 2
                        i32.load offset=4
                        local.tee 6
                        i32.const 3
                        i32.and
                        i32.const 1
                        i32.ne
                        br_if 0 (;@9;)
                        local.get 2
                        local.get 6
                        i32.const -8
                        i32.and
                        local.tee 6
                        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
                        local.get 6
                        local.get 3
                        i32.add
                        local.set 3
                        local.get 2
                        local.get 6
                        i32.add
                        local.tee 2
                        i32.load offset=4
                        local.set 6
                      end
                      local.get 2
                      local.get 6
                      i32.const -2
                      i32.and
                      i32.store offset=4
                      local.get 0
                      local.get 3
                      i32.const 1
                      i32.or
                      i32.store offset=4
                      local.get 0
                      local.get 3
                      i32.add
                      local.get 3
                      i32.store
                      block ;; label = @9
                        local.get 3
                        i32.const 256
                        i32.lt_u
                        br_if 0 (;@9;)
                        local.get 0
                        local.get 3
                        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std
                        br 6 (;@3;)
                      end
                      block ;; label = @9
                        block ;; label = @10
                          i32.const 0
                          i32.load offset=1049684
                          local.tee 2
                          i32.const 1
                          local.get 3
                          i32.const 3
                          i32.shr_u
                          i32.shl
                          local.tee 6
                          i32.and
                          br_if 0 (;@10;)
                          i32.const 0
                          local.get 2
                          local.get 6
                          i32.or
                          i32.store offset=1049684
                          local.get 3
                          i32.const 248
                          i32.and
                          i32.const 1049420
                          i32.add
                          local.tee 3
                          local.set 2
                          br 1 (;@9;)
                        end
                        local.get 3
                        i32.const 248
                        i32.and
                        local.tee 3
                        i32.const 1049420
                        i32.add
                        local.set 2
                        local.get 3
                        i32.const 1049428
                        i32.add
                        i32.load
                        local.set 3
                      end
                      local.get 2
                      local.get 0
                      i32.store offset=8
                      local.get 3
                      local.get 0
                      i32.store offset=12
                      local.get 0
                      local.get 2
                      i32.store offset=12
                      local.get 0
                      local.get 3
                      i32.store offset=8
                      br 5 (;@3;)
                    end
                    i32.const 0
                    local.get 0
                    local.get 3
                    i32.sub
                    local.tee 2
                    i32.store offset=1049696
                    i32.const 0
                    i32.const 0
                    i32.load offset=1049704
                    local.tee 0
                    local.get 3
                    i32.add
                    local.tee 8
                    i32.store offset=1049704
                    local.get 8
                    local.get 2
                    i32.const 1
                    i32.or
                    i32.store offset=4
                    local.get 0
                    local.get 3
                    i32.const 3
                    i32.or
                    i32.store offset=4
                    local.get 0
                    i32.const 8
                    i32.add
                    local.set 0
                    br 6 (;@1;)
                  end
                  i32.const 0
                  i32.load offset=1049700
                  local.set 2
                  block ;; label = @7
                    block ;; label = @8
                      local.get 0
                      local.get 3
                      i32.sub
                      local.tee 8
                      i32.const 15
                      i32.gt_u
                      br_if 0 (;@8;)
                      i32.const 0
                      i32.const 0
                      i32.store offset=1049700
                      i32.const 0
                      i32.const 0
                      i32.store offset=1049692
                      local.get 2
                      local.get 0
                      i32.const 3
                      i32.or
                      i32.store offset=4
                      local.get 2
                      local.get 0
                      i32.add
                      local.tee 0
                      local.get 0
                      i32.load offset=4
                      i32.const 1
                      i32.or
                      i32.store offset=4
                      br 1 (;@7;)
                    end
                    i32.const 0
                    local.get 8
                    i32.store offset=1049692
                    i32.const 0
                    local.get 2
                    local.get 3
                    i32.add
                    local.tee 6
                    i32.store offset=1049700
                    local.get 6
                    local.get 8
                    i32.const 1
                    i32.or
                    i32.store offset=4
                    local.get 2
                    local.get 0
                    i32.add
                    local.get 8
                    i32.store
                    local.get 2
                    local.get 3
                    i32.const 3
                    i32.or
                    i32.store offset=4
                  end
                  local.get 2
                  i32.const 8
                  i32.add
                  local.set 0
                  br 5 (;@1;)
                end
                local.get 0
                local.get 7
                local.get 9
                i32.add
                i32.store offset=4
                i32.const 0
                i32.const 0
                i32.load offset=1049704
                local.tee 0
                i32.const 15
                i32.add
                i32.const -8
                i32.and
                local.tee 2
                i32.const -8
                i32.add
                local.tee 8
                i32.store offset=1049704
                i32.const 0
                local.get 0
                local.get 2
                i32.sub
                i32.const 0
                i32.load offset=1049696
                local.get 9
                i32.add
                local.tee 2
                i32.add
                i32.const 8
                i32.add
                local.tee 6
                i32.store offset=1049696
                local.get 8
                local.get 6
                i32.const 1
                i32.or
                i32.store offset=4
                local.get 0
                local.get 2
                i32.add
                i32.const 40
                i32.store offset=4
                i32.const 0
                i32.const 2097152
                i32.store offset=1049716
                br 3 (;@2;)
              end
              i32.const 0
              local.get 0
              i32.store offset=1049704
              i32.const 0
              i32.const 0
              i32.load offset=1049696
              local.get 3
              i32.add
              local.tee 3
              i32.store offset=1049696
              local.get 0
              local.get 3
              i32.const 1
              i32.or
              i32.store offset=4
              br 1 (;@3;)
            end
            i32.const 0
            local.get 0
            i32.store offset=1049700
            i32.const 0
            i32.const 0
            i32.load offset=1049692
            local.get 3
            i32.add
            local.tee 3
            i32.store offset=1049692
            local.get 0
            local.get 3
            i32.const 1
            i32.or
            i32.store offset=4
            local.get 0
            local.get 3
            i32.add
            local.get 3
            i32.store
          end
          local.get 8
          i32.const 8
          i32.add
          local.set 0
          br 1 (;@1;)
        end
        i32.const 0
        local.set 0
        i32.const 0
        i32.load offset=1049696
        local.tee 2
        local.get 3
        i32.le_u
        br_if 0 (;@1;)
        i32.const 0
        local.get 2
        local.get 3
        i32.sub
        local.tee 2
        i32.store offset=1049696
        i32.const 0
        i32.const 0
        i32.load offset=1049704
        local.tee 0
        local.get 3
        i32.add
        local.tee 8
        i32.store offset=1049704
        local.get 8
        local.get 2
        i32.const 1
        i32.or
        i32.store offset=4
        local.get 0
        local.get 3
        i32.const 3
        i32.or
        i32.store offset=4
        local.get 0
        i32.const 8
        i32.add
        local.set 0
      end
      local.get 1
      i32.const 16
      i32.add
      global.set $__stack_pointer
      local.get 0
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc12___rust_abort (;27;) (type 3)
      unreachable
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc13___rdl_dealloc (;28;) (type 5) (param i32 i32 i32)
      (local i32 i32)
      block ;; label = @1
        block ;; label = @2
          local.get 0
          i32.const -4
          i32.add
          i32.load
          local.tee 3
          i32.const -8
          i32.and
          local.tee 4
          i32.const 4
          i32.const 8
          local.get 3
          i32.const 3
          i32.and
          local.tee 3
          select
          local.get 1
          i32.add
          i32.lt_u
          br_if 0 (;@2;)
          block ;; label = @3
            local.get 3
            i32.eqz
            br_if 0 (;@3;)
            local.get 4
            local.get 1
            i32.const 39
            i32.add
            i32.gt_u
            br_if 2 (;@1;)
          end
          local.get 0
          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE4freeCsdl5sGgnNXvY_3std
          return
        end
        i32.const 1048932
        i32.const 46
        i32.const 1048980
        call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
        unreachable
      end
      i32.const 1048996
      i32.const 46
      i32.const 1049044
      call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
      unreachable
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE4freeCsdl5sGgnNXvY_3std (;29;) (type 4) (param i32)
      (local i32 i32 i32 i32)
      local.get 0
      i32.const -8
      i32.add
      local.tee 1
      local.get 0
      i32.const -4
      i32.add
      i32.load
      local.tee 2
      i32.const -8
      i32.and
      local.tee 0
      i32.add
      local.set 3
      block ;; label = @1
        block ;; label = @2
          local.get 2
          i32.const 1
          i32.and
          br_if 0 (;@2;)
          local.get 2
          i32.const 2
          i32.and
          i32.eqz
          br_if 1 (;@1;)
          local.get 1
          i32.load
          local.tee 2
          local.get 0
          i32.add
          local.set 0
          block ;; label = @3
            local.get 1
            local.get 2
            i32.sub
            local.tee 1
            i32.const 0
            i32.load offset=1049700
            i32.ne
            br_if 0 (;@3;)
            local.get 3
            i32.load offset=4
            i32.const 3
            i32.and
            i32.const 3
            i32.ne
            br_if 1 (;@2;)
            i32.const 0
            local.get 0
            i32.store offset=1049692
            local.get 3
            local.get 3
            i32.load offset=4
            i32.const -2
            i32.and
            i32.store offset=4
            local.get 1
            local.get 0
            i32.const 1
            i32.or
            i32.store offset=4
            local.get 3
            local.get 0
            i32.store
            return
          end
          local.get 1
          local.get 2
          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
        end
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    block ;; label = @8
                      block ;; label = @9
                        local.get 3
                        i32.load offset=4
                        local.tee 2
                        i32.const 2
                        i32.and
                        br_if 0 (;@9;)
                        local.get 3
                        i32.const 0
                        i32.load offset=1049704
                        i32.eq
                        br_if 2 (;@7;)
                        local.get 3
                        i32.const 0
                        i32.load offset=1049700
                        i32.eq
                        br_if 3 (;@6;)
                        local.get 3
                        local.get 2
                        i32.const -8
                        i32.and
                        local.tee 2
                        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
                        local.get 1
                        local.get 2
                        local.get 0
                        i32.add
                        local.tee 0
                        i32.const 1
                        i32.or
                        i32.store offset=4
                        local.get 1
                        local.get 0
                        i32.add
                        local.get 0
                        i32.store
                        local.get 1
                        i32.const 0
                        i32.load offset=1049700
                        i32.ne
                        br_if 1 (;@8;)
                        i32.const 0
                        local.get 0
                        i32.store offset=1049692
                        return
                      end
                      local.get 3
                      local.get 2
                      i32.const -2
                      i32.and
                      i32.store offset=4
                      local.get 1
                      local.get 0
                      i32.const 1
                      i32.or
                      i32.store offset=4
                      local.get 1
                      local.get 0
                      i32.add
                      local.get 0
                      i32.store
                    end
                    local.get 0
                    i32.const 256
                    i32.lt_u
                    br_if 4 (;@3;)
                    local.get 1
                    local.get 0
                    call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std
                    i32.const 0
                    i32.const 0
                    i32.load offset=1049724
                    i32.const -1
                    i32.add
                    local.tee 1
                    i32.store offset=1049724
                    local.get 1
                    br_if 6 (;@1;)
                    i32.const 0
                    i32.load offset=1049412
                    local.tee 0
                    br_if 2 (;@5;)
                    i32.const 4095
                    local.set 1
                    br 3 (;@4;)
                  end
                  i32.const 0
                  local.get 1
                  i32.store offset=1049704
                  i32.const 0
                  i32.const 0
                  i32.load offset=1049696
                  local.get 0
                  i32.add
                  local.tee 0
                  i32.store offset=1049696
                  local.get 1
                  local.get 0
                  i32.const 1
                  i32.or
                  i32.store offset=4
                  block ;; label = @7
                    local.get 1
                    i32.const 0
                    i32.load offset=1049700
                    i32.ne
                    br_if 0 (;@7;)
                    i32.const 0
                    i32.const 0
                    i32.store offset=1049692
                    i32.const 0
                    i32.const 0
                    i32.store offset=1049700
                  end
                  local.get 0
                  i32.const 0
                  i32.load offset=1049716
                  local.tee 2
                  i32.le_u
                  br_if 5 (;@1;)
                  i32.const 0
                  i32.load offset=1049704
                  local.tee 0
                  i32.eqz
                  br_if 5 (;@1;)
                  i32.const 0
                  i32.load offset=1049696
                  local.tee 4
                  i32.const 41
                  i32.lt_u
                  br_if 4 (;@2;)
                  i32.const 1049404
                  local.set 1
                  loop ;; label = @7
                    block ;; label = @8
                      local.get 1
                      i32.load
                      local.tee 3
                      local.get 0
                      i32.gt_u
                      br_if 0 (;@8;)
                      local.get 0
                      local.get 3
                      local.get 1
                      i32.load offset=4
                      i32.add
                      i32.lt_u
                      br_if 6 (;@2;)
                    end
                    local.get 1
                    i32.load offset=8
                    local.set 1
                    br 0 (;@7;)
                  end
                end
                i32.const 0
                local.get 1
                i32.store offset=1049700
                i32.const 0
                i32.const 0
                i32.load offset=1049692
                local.get 0
                i32.add
                local.tee 0
                i32.store offset=1049692
                local.get 1
                local.get 0
                i32.const 1
                i32.or
                i32.store offset=4
                local.get 1
                local.get 0
                i32.add
                local.get 0
                i32.store
                return
              end
              i32.const 0
              local.set 1
              loop ;; label = @5
                local.get 1
                i32.const 1
                i32.add
                local.set 1
                local.get 0
                i32.load offset=8
                local.tee 0
                br_if 0 (;@5;)
              end
              local.get 1
              i32.const 4095
              local.get 1
              i32.const 4095
              i32.gt_u
              select
              local.set 1
            end
            i32.const 0
            local.get 1
            i32.store offset=1049724
            return
          end
          block ;; label = @3
            block ;; label = @4
              i32.const 0
              i32.load offset=1049684
              local.tee 3
              i32.const 1
              local.get 0
              i32.const 3
              i32.shr_u
              i32.shl
              local.tee 2
              i32.and
              br_if 0 (;@4;)
              i32.const 0
              local.get 3
              local.get 2
              i32.or
              i32.store offset=1049684
              local.get 0
              i32.const 248
              i32.and
              i32.const 1049420
              i32.add
              local.tee 0
              local.set 3
              br 1 (;@3;)
            end
            local.get 0
            i32.const 248
            i32.and
            local.tee 0
            i32.const 1049420
            i32.add
            local.set 3
            local.get 0
            i32.const 1049428
            i32.add
            i32.load
            local.set 0
          end
          local.get 3
          local.get 1
          i32.store offset=8
          local.get 0
          local.get 1
          i32.store offset=12
          local.get 1
          local.get 3
          i32.store offset=12
          local.get 1
          local.get 0
          i32.store offset=8
          return
        end
        block ;; label = @2
          block ;; label = @3
            i32.const 0
            i32.load offset=1049412
            local.tee 0
            br_if 0 (;@3;)
            i32.const 4095
            local.set 1
            br 1 (;@2;)
          end
          i32.const 0
          local.set 1
          loop ;; label = @3
            local.get 1
            i32.const 1
            i32.add
            local.set 1
            local.get 0
            i32.load offset=8
            local.tee 0
            br_if 0 (;@3;)
          end
          local.get 1
          i32.const 4095
          local.get 1
          i32.const 4095
          i32.gt_u
          select
          local.set 1
        end
        i32.const 0
        local.get 1
        i32.store offset=1049724
        local.get 4
        local.get 2
        i32.le_u
        br_if 0 (;@1;)
        i32.const 0
        i32.const -1
        i32.store offset=1049716
      end
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc13___rdl_realloc (;30;) (type 6) (param i32 i32 i32 i32) (result i32)
      (local i32 i32 i32 i32 i32 i32)
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    block ;; label = @8
                      local.get 0
                      i32.const -4
                      i32.add
                      local.tee 4
                      i32.load
                      local.tee 5
                      i32.const -8
                      i32.and
                      local.tee 6
                      i32.const 4
                      i32.const 8
                      local.get 5
                      i32.const 3
                      i32.and
                      local.tee 7
                      select
                      local.get 1
                      i32.add
                      i32.lt_u
                      br_if 0 (;@8;)
                      local.get 1
                      i32.const 39
                      i32.add
                      local.set 8
                      block ;; label = @9
                        local.get 7
                        i32.eqz
                        br_if 0 (;@9;)
                        local.get 6
                        local.get 8
                        i32.gt_u
                        br_if 2 (;@7;)
                      end
                      block ;; label = @9
                        block ;; label = @10
                          local.get 2
                          i32.const 9
                          i32.lt_u
                          br_if 0 (;@10;)
                          local.get 2
                          local.get 3
                          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE8memalignCsdl5sGgnNXvY_3std
                          local.tee 2
                          br_if 1 (;@9;)
                          i32.const 0
                          return
                        end
                        i32.const 0
                        local.set 2
                        local.get 3
                        i32.const -65588
                        i32.gt_u
                        br_if 8 (;@1;)
                        i32.const 16
                        local.get 3
                        i32.const 11
                        i32.add
                        i32.const -8
                        i32.and
                        local.get 3
                        i32.const 11
                        i32.lt_u
                        select
                        local.set 1
                        local.get 0
                        i32.const -8
                        i32.add
                        local.set 8
                        block ;; label = @10
                          local.get 7
                          br_if 0 (;@10;)
                          local.get 1
                          i32.const 256
                          i32.lt_u
                          br_if 7 (;@3;)
                          local.get 8
                          i32.eqz
                          br_if 7 (;@3;)
                          local.get 6
                          local.get 1
                          i32.le_u
                          br_if 7 (;@3;)
                          local.get 6
                          local.get 1
                          i32.sub
                          i32.const 131072
                          i32.gt_u
                          br_if 7 (;@3;)
                          local.get 0
                          return
                        end
                        local.get 8
                        local.get 6
                        i32.add
                        local.set 7
                        block ;; label = @10
                          block ;; label = @11
                            local.get 6
                            local.get 1
                            i32.ge_u
                            br_if 0 (;@11;)
                            local.get 7
                            i32.const 0
                            i32.load offset=1049704
                            i32.eq
                            br_if 1 (;@10;)
                            block ;; label = @12
                              local.get 7
                              i32.const 0
                              i32.load offset=1049700
                              i32.eq
                              br_if 0 (;@12;)
                              local.get 7
                              i32.load offset=4
                              local.tee 5
                              i32.const 2
                              i32.and
                              br_if 9 (;@3;)
                              local.get 5
                              i32.const -8
                              i32.and
                              local.tee 9
                              local.get 6
                              i32.add
                              local.tee 5
                              local.get 1
                              i32.lt_u
                              br_if 9 (;@3;)
                              local.get 7
                              local.get 9
                              call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
                              block ;; label = @13
                                local.get 5
                                local.get 1
                                i32.sub
                                local.tee 7
                                i32.const 16
                                i32.lt_u
                                br_if 0 (;@13;)
                                local.get 4
                                local.get 1
                                local.get 4
                                i32.load
                                i32.const 1
                                i32.and
                                i32.or
                                i32.const 2
                                i32.or
                                i32.store
                                local.get 8
                                local.get 1
                                i32.add
                                local.tee 1
                                local.get 7
                                i32.const 3
                                i32.or
                                i32.store offset=4
                                local.get 8
                                local.get 5
                                i32.add
                                local.tee 5
                                local.get 5
                                i32.load offset=4
                                i32.const 1
                                i32.or
                                i32.store offset=4
                                local.get 1
                                local.get 7
                                call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE13dispose_chunkCsdl5sGgnNXvY_3std
                                br 9 (;@4;)
                              end
                              local.get 4
                              local.get 5
                              local.get 4
                              i32.load
                              i32.const 1
                              i32.and
                              i32.or
                              i32.const 2
                              i32.or
                              i32.store
                              local.get 8
                              local.get 5
                              i32.add
                              local.tee 1
                              local.get 1
                              i32.load offset=4
                              i32.const 1
                              i32.or
                              i32.store offset=4
                              br 8 (;@4;)
                            end
                            i32.const 0
                            i32.load offset=1049692
                            local.get 6
                            i32.add
                            local.tee 7
                            local.get 1
                            i32.lt_u
                            br_if 8 (;@3;)
                            block ;; label = @12
                              block ;; label = @13
                                local.get 7
                                local.get 1
                                i32.sub
                                local.tee 6
                                i32.const 15
                                i32.gt_u
                                br_if 0 (;@13;)
                                local.get 4
                                local.get 5
                                i32.const 1
                                i32.and
                                local.get 7
                                i32.or
                                i32.const 2
                                i32.or
                                i32.store
                                local.get 8
                                local.get 7
                                i32.add
                                local.tee 1
                                local.get 1
                                i32.load offset=4
                                i32.const 1
                                i32.or
                                i32.store offset=4
                                i32.const 0
                                local.set 6
                                i32.const 0
                                local.set 1
                                br 1 (;@12;)
                              end
                              local.get 4
                              local.get 1
                              local.get 5
                              i32.const 1
                              i32.and
                              i32.or
                              i32.const 2
                              i32.or
                              i32.store
                              local.get 8
                              local.get 1
                              i32.add
                              local.tee 1
                              local.get 6
                              i32.const 1
                              i32.or
                              i32.store offset=4
                              local.get 8
                              local.get 7
                              i32.add
                              local.tee 7
                              local.get 6
                              i32.store
                              local.get 7
                              local.get 7
                              i32.load offset=4
                              i32.const -2
                              i32.and
                              i32.store offset=4
                            end
                            i32.const 0
                            local.get 1
                            i32.store offset=1049700
                            i32.const 0
                            local.get 6
                            i32.store offset=1049692
                            br 7 (;@4;)
                          end
                          local.get 6
                          local.get 1
                          i32.sub
                          local.tee 6
                          i32.const 15
                          i32.le_u
                          br_if 6 (;@4;)
                          local.get 4
                          local.get 1
                          local.get 5
                          i32.const 1
                          i32.and
                          i32.or
                          i32.const 2
                          i32.or
                          i32.store
                          local.get 8
                          local.get 1
                          i32.add
                          local.tee 1
                          local.get 6
                          i32.const 3
                          i32.or
                          i32.store offset=4
                          local.get 7
                          local.get 7
                          i32.load offset=4
                          i32.const 1
                          i32.or
                          i32.store offset=4
                          local.get 1
                          local.get 6
                          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE13dispose_chunkCsdl5sGgnNXvY_3std
                          br 6 (;@4;)
                        end
                        i32.const 0
                        i32.load offset=1049696
                        local.get 6
                        i32.add
                        local.tee 7
                        local.get 1
                        i32.gt_u
                        br_if 4 (;@5;)
                        br 6 (;@3;)
                      end
                      block ;; label = @9
                        local.get 3
                        local.get 1
                        local.get 3
                        local.get 1
                        i32.lt_u
                        select
                        local.tee 3
                        i32.eqz
                        br_if 0 (;@9;)
                        local.get 2
                        local.get 0
                        local.get 3
                        memory.copy
                      end
                      local.get 4
                      i32.load
                      local.tee 3
                      i32.const -8
                      i32.and
                      local.tee 7
                      i32.const 4
                      i32.const 8
                      local.get 3
                      i32.const 3
                      i32.and
                      local.tee 3
                      select
                      local.get 1
                      i32.add
                      i32.lt_u
                      br_if 2 (;@6;)
                      local.get 3
                      i32.eqz
                      br_if 6 (;@2;)
                      local.get 7
                      local.get 8
                      i32.le_u
                      br_if 6 (;@2;)
                      i32.const 1048996
                      i32.const 46
                      i32.const 1049044
                      call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
                      unreachable
                    end
                    i32.const 1048932
                    i32.const 46
                    i32.const 1048980
                    call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
                    unreachable
                  end
                  i32.const 1048996
                  i32.const 46
                  i32.const 1049044
                  call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
                  unreachable
                end
                i32.const 1048932
                i32.const 46
                i32.const 1048980
                call $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic
                unreachable
              end
              local.get 4
              local.get 1
              local.get 5
              i32.const 1
              i32.and
              i32.or
              i32.const 2
              i32.or
              i32.store
              local.get 8
              local.get 1
              i32.add
              local.tee 5
              local.get 7
              local.get 1
              i32.sub
              local.tee 1
              i32.const 1
              i32.or
              i32.store offset=4
              i32.const 0
              local.get 1
              i32.store offset=1049696
              i32.const 0
              local.get 5
              i32.store offset=1049704
            end
            local.get 8
            i32.eqz
            br_if 0 (;@3;)
            local.get 0
            return
          end
          local.get 3
          call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE6mallocCsdl5sGgnNXvY_3std
          local.tee 1
          i32.eqz
          br_if 1 (;@1;)
          block ;; label = @3
            local.get 3
            i32.const -4
            i32.const -8
            local.get 4
            i32.load
            local.tee 2
            i32.const 3
            i32.and
            select
            local.get 2
            i32.const -8
            i32.and
            i32.add
            local.tee 2
            local.get 3
            local.get 2
            i32.lt_u
            select
            local.tee 3
            i32.eqz
            br_if 0 (;@3;)
            local.get 1
            local.get 0
            local.get 3
            memory.copy
          end
          local.get 1
          local.set 2
        end
        local.get 0
        call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE4freeCsdl5sGgnNXvY_3std
      end
      local.get 2
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std (;31;) (type 0) (param i32 i32)
      (local i32 i32 i32 i32)
      local.get 0
      i32.load offset=12
      local.set 2
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            block ;; label = @4
              local.get 1
              i32.const 256
              i32.lt_u
              br_if 0 (;@4;)
              local.get 0
              i32.load offset=24
              local.set 3
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    local.get 2
                    local.get 0
                    i32.ne
                    br_if 0 (;@7;)
                    local.get 0
                    i32.const 20
                    i32.const 16
                    local.get 0
                    i32.load offset=20
                    local.tee 2
                    select
                    i32.add
                    i32.load
                    local.tee 1
                    br_if 1 (;@6;)
                    i32.const 0
                    local.set 2
                    br 2 (;@5;)
                  end
                  local.get 0
                  i32.load offset=8
                  local.tee 1
                  local.get 2
                  i32.store offset=12
                  local.get 2
                  local.get 1
                  i32.store offset=8
                  br 1 (;@5;)
                end
                local.get 0
                i32.const 20
                i32.add
                local.get 0
                i32.const 16
                i32.add
                local.get 2
                select
                local.set 4
                loop ;; label = @6
                  local.get 4
                  local.set 5
                  local.get 1
                  local.tee 2
                  i32.const 20
                  i32.add
                  local.get 2
                  i32.const 16
                  i32.add
                  local.get 2
                  i32.load offset=20
                  local.tee 1
                  select
                  local.set 4
                  local.get 2
                  i32.const 20
                  i32.const 16
                  local.get 1
                  select
                  i32.add
                  i32.load
                  local.tee 1
                  br_if 0 (;@6;)
                end
                local.get 5
                i32.const 0
                i32.store
              end
              local.get 3
              i32.eqz
              br_if 2 (;@2;)
              block ;; label = @5
                block ;; label = @6
                  local.get 0
                  local.get 0
                  i32.load offset=28
                  i32.const 2
                  i32.shl
                  i32.const 1049276
                  i32.add
                  local.tee 1
                  i32.load
                  i32.eq
                  br_if 0 (;@6;)
                  local.get 3
                  i32.load offset=16
                  local.get 0
                  i32.eq
                  br_if 1 (;@5;)
                  local.get 3
                  local.get 2
                  i32.store offset=20
                  local.get 2
                  br_if 3 (;@3;)
                  br 4 (;@2;)
                end
                local.get 1
                local.get 2
                i32.store
                local.get 2
                i32.eqz
                br_if 4 (;@1;)
                br 2 (;@3;)
              end
              local.get 3
              local.get 2
              i32.store offset=16
              local.get 2
              br_if 1 (;@3;)
              br 2 (;@2;)
            end
            block ;; label = @4
              local.get 2
              local.get 0
              i32.load offset=8
              local.tee 4
              i32.eq
              br_if 0 (;@4;)
              local.get 4
              local.get 2
              i32.store offset=12
              local.get 2
              local.get 4
              i32.store offset=8
              return
            end
            i32.const 0
            i32.const 0
            i32.load offset=1049684
            i32.const -2
            local.get 1
            i32.const 3
            i32.shr_u
            i32.rotl
            i32.and
            i32.store offset=1049684
            return
          end
          local.get 2
          local.get 3
          i32.store offset=24
          block ;; label = @3
            local.get 0
            i32.load offset=16
            local.tee 1
            i32.eqz
            br_if 0 (;@3;)
            local.get 2
            local.get 1
            i32.store offset=16
            local.get 1
            local.get 2
            i32.store offset=24
          end
          local.get 0
          i32.load offset=20
          local.tee 1
          i32.eqz
          br_if 0 (;@2;)
          local.get 2
          local.get 1
          i32.store offset=20
          local.get 1
          local.get 2
          i32.store offset=24
          return
        end
        return
      end
      i32.const 0
      i32.const 0
      i32.load offset=1049688
      i32.const -2
      local.get 0
      i32.load offset=28
      i32.rotl
      i32.and
      i32.store offset=1049688
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE13dispose_chunkCsdl5sGgnNXvY_3std (;32;) (type 0) (param i32 i32)
      (local i32 i32)
      local.get 0
      local.get 1
      i32.add
      local.set 2
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 0
            i32.load offset=4
            local.tee 3
            i32.const 1
            i32.and
            br_if 0 (;@3;)
            local.get 3
            i32.const 2
            i32.and
            i32.eqz
            br_if 1 (;@2;)
            local.get 0
            i32.load
            local.tee 3
            local.get 1
            i32.add
            local.set 1
            block ;; label = @4
              local.get 0
              local.get 3
              i32.sub
              local.tee 0
              i32.const 0
              i32.load offset=1049700
              i32.ne
              br_if 0 (;@4;)
              local.get 2
              i32.load offset=4
              i32.const 3
              i32.and
              i32.const 3
              i32.ne
              br_if 1 (;@3;)
              i32.const 0
              local.get 1
              i32.store offset=1049692
              local.get 2
              local.get 2
              i32.load offset=4
              i32.const -2
              i32.and
              i32.store offset=4
              local.get 0
              local.get 1
              i32.const 1
              i32.or
              i32.store offset=4
              local.get 2
              local.get 1
              i32.store
              return
            end
            local.get 0
            local.get 3
            call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
          end
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                local.get 2
                i32.load offset=4
                local.tee 3
                i32.const 2
                i32.and
                br_if 0 (;@5;)
                local.get 2
                i32.const 0
                i32.load offset=1049704
                i32.eq
                br_if 2 (;@3;)
                local.get 2
                i32.const 0
                i32.load offset=1049700
                i32.eq
                br_if 4 (;@1;)
                local.get 2
                local.get 3
                i32.const -8
                i32.and
                local.tee 3
                call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE12unlink_chunkCsdl5sGgnNXvY_3std
                local.get 0
                local.get 3
                local.get 1
                i32.add
                local.tee 1
                i32.const 1
                i32.or
                i32.store offset=4
                local.get 0
                local.get 1
                i32.add
                local.get 1
                i32.store
                local.get 0
                i32.const 0
                i32.load offset=1049700
                i32.ne
                br_if 1 (;@4;)
                i32.const 0
                local.get 1
                i32.store offset=1049692
                return
              end
              local.get 2
              local.get 3
              i32.const -2
              i32.and
              i32.store offset=4
              local.get 0
              local.get 1
              i32.const 1
              i32.or
              i32.store offset=4
              local.get 0
              local.get 1
              i32.add
              local.get 1
              i32.store
            end
            block ;; label = @4
              local.get 1
              i32.const 256
              i32.lt_u
              br_if 0 (;@4;)
              local.get 0
              local.get 1
              call $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std
              return
            end
            block ;; label = @4
              block ;; label = @5
                i32.const 0
                i32.load offset=1049684
                local.tee 2
                i32.const 1
                local.get 1
                i32.const 3
                i32.shr_u
                i32.shl
                local.tee 3
                i32.and
                br_if 0 (;@5;)
                i32.const 0
                local.get 2
                local.get 3
                i32.or
                i32.store offset=1049684
                local.get 1
                i32.const 248
                i32.and
                i32.const 1049420
                i32.add
                local.tee 1
                local.set 2
                br 1 (;@4;)
              end
              local.get 1
              i32.const 248
              i32.and
              local.tee 1
              i32.const 1049420
              i32.add
              local.set 2
              local.get 1
              i32.const 1049428
              i32.add
              i32.load
              local.set 1
            end
            local.get 2
            local.get 0
            i32.store offset=8
            local.get 1
            local.get 0
            i32.store offset=12
            local.get 0
            local.get 2
            i32.store offset=12
            local.get 0
            local.get 1
            i32.store offset=8
            return
          end
          i32.const 0
          local.get 0
          i32.store offset=1049704
          i32.const 0
          i32.const 0
          i32.load offset=1049696
          local.get 1
          i32.add
          local.tee 1
          i32.store offset=1049696
          local.get 0
          local.get 1
          i32.const 1
          i32.or
          i32.store offset=4
          local.get 0
          i32.const 0
          i32.load offset=1049700
          i32.ne
          br_if 0 (;@2;)
          i32.const 0
          i32.const 0
          i32.store offset=1049692
          i32.const 0
          i32.const 0
          i32.store offset=1049700
        end
        return
      end
      i32.const 0
      local.get 0
      i32.store offset=1049700
      i32.const 0
      i32.const 0
      i32.load offset=1049692
      local.get 1
      i32.add
      local.tee 1
      i32.store offset=1049692
      local.get 0
      local.get 1
      i32.const 1
      i32.or
      i32.store offset=4
      local.get 0
      local.get 1
      i32.add
      local.get 1
      i32.store
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc17rust_begin_unwind (;33;) (type 4) (param i32)
      (local i32 i64)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 1
      global.set $__stack_pointer
      local.get 0
      i64.load align=4
      local.set 2
      local.get 1
      local.get 0
      i32.store offset=12
      local.get 1
      local.get 2
      i64.store offset=4 align=4
      local.get 1
      i32.const 4
      i32.add
      call $_RINvNtNtCsdl5sGgnNXvY_3std3sys9backtrace26___rust_end_short_backtraceNCNvNtB6_9panicking13panic_handler0zEB6_
      unreachable
    )
    (func $_RNvCs9wFQrvczXsK_7___rustc26___rust_alloc_error_handler (;34;) (type 0) (param i32 i32)
      local.get 1
      local.get 0
      call $_RNvNtCsdl5sGgnNXvY_3std5alloc8rust_oom
      unreachable
    )
    (func $_RNvNtCsdl5sGgnNXvY_3std5alloc8rust_oom (;35;) (type 0) (param i32 i32)
      (local i32)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 2
      global.set $__stack_pointer
      local.get 2
      local.get 1
      i32.store offset=12
      local.get 2
      local.get 0
      i32.store offset=8
      local.get 2
      i32.const 8
      i32.add
      call $_RINvNtNtCsdl5sGgnNXvY_3std3sys9backtrace26___rust_end_short_backtraceNCNvNtB6_5alloc8rust_oom0zEB6_
      unreachable
    )
    (func $_RNvMs0_NtCsgis5MWNmFLl_8dlmalloc8dlmallocINtB5_8DlmallocNtNtB7_3sys6SystemE18insert_large_chunkCsdl5sGgnNXvY_3std (;36;) (type 0) (param i32 i32)
      (local i32 i32 i32 i32)
      i32.const 0
      local.set 2
      block ;; label = @1
        local.get 1
        i32.const 8
        i32.shr_u
        local.tee 3
        i32.eqz
        br_if 0 (;@1;)
        i32.const 31
        local.set 2
        local.get 1
        i32.const 16777216
        i32.ge_u
        br_if 0 (;@1;)
        local.get 1
        i32.const 38
        local.get 3
        i32.clz
        local.tee 2
        i32.sub
        i32.shr_u
        i32.const 1
        i32.and
        local.get 2
        i32.const 1
        i32.shl
        i32.or
        i32.const 62
        i32.xor
        local.set 2
      end
      local.get 0
      i64.const 0
      i64.store offset=16 align=4
      local.get 0
      local.get 2
      i32.store offset=28
      local.get 2
      i32.const 2
      i32.shl
      i32.const 1049276
      i32.add
      local.set 3
      block ;; label = @1
        i32.const 0
        i32.load offset=1049688
        i32.const 1
        local.get 2
        i32.shl
        local.tee 4
        i32.and
        br_if 0 (;@1;)
        local.get 3
        local.get 0
        i32.store
        local.get 0
        local.get 3
        i32.store offset=24
        local.get 0
        local.get 0
        i32.store offset=12
        local.get 0
        local.get 0
        i32.store offset=8
        i32.const 0
        i32.const 0
        i32.load offset=1049688
        local.get 4
        i32.or
        i32.store offset=1049688
        return
      end
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 3
            i32.load
            local.tee 4
            i32.load offset=4
            i32.const -8
            i32.and
            local.get 1
            i32.ne
            br_if 0 (;@3;)
            local.get 4
            local.set 2
            br 1 (;@2;)
          end
          local.get 1
          i32.const 0
          i32.const 25
          local.get 2
          i32.const 1
          i32.shr_u
          i32.sub
          local.get 2
          i32.const 31
          i32.eq
          select
          i32.shl
          local.set 3
          loop ;; label = @3
            local.get 4
            local.get 3
            i32.const 29
            i32.shr_u
            i32.const 4
            i32.and
            i32.add
            local.tee 5
            i32.load offset=16
            local.tee 2
            i32.eqz
            br_if 2 (;@1;)
            local.get 3
            i32.const 1
            i32.shl
            local.set 3
            local.get 2
            local.set 4
            local.get 2
            i32.load offset=4
            i32.const -8
            i32.and
            local.get 1
            i32.ne
            br_if 0 (;@3;)
          end
        end
        local.get 2
        i32.load offset=8
        local.tee 3
        local.get 0
        i32.store offset=12
        local.get 2
        local.get 0
        i32.store offset=8
        local.get 0
        i32.const 0
        i32.store offset=24
        local.get 0
        local.get 2
        i32.store offset=12
        local.get 0
        local.get 3
        i32.store offset=8
        return
      end
      local.get 5
      i32.const 16
      i32.add
      local.get 0
      i32.store
      local.get 0
      local.get 4
      i32.store offset=24
      local.get 0
      local.get 0
      i32.store offset=12
      local.get 0
      local.get 0
      i32.store offset=8
    )
    (func $_RNvNtNtCsdl5sGgnNXvY_3std9panicking11panic_count8increase (;37;) (type 9) (param i32) (result i32)
      (local i32 i32)
      i32.const 0
      local.set 1
      i32.const 0
      i32.const 0
      i32.load offset=1049272
      local.tee 2
      i32.const 1
      i32.add
      i32.store offset=1049272
      block ;; label = @1
        local.get 2
        i32.const 0
        i32.lt_s
        br_if 0 (;@1;)
        i32.const 1
        local.set 1
        i32.const 0
        i32.load8_u offset=1049252
        br_if 0 (;@1;)
        i32.const 0
        local.get 0
        i32.store8 offset=1049252
        i32.const 0
        i32.const 0
        i32.load offset=1049248
        i32.const 1
        i32.add
        i32.store offset=1049248
        i32.const 2
        local.set 1
      end
      local.get 1
    )
    (func $_RNvXNtCsdkdt1aaAg1T_4core3anyNtNtCsewWLk9TkM7w_5alloc6string6StringNtB2_3Any7type_idCsdl5sGgnNXvY_3std (;38;) (type 0) (param i32 i32)
      local.get 0
      i32.const 0
      i64.load offset=1048704 align=4
      i64.store offset=8 align=4
      local.get 0
      i32.const 0
      i64.load offset=1048696 align=4
      i64.store align=4
    )
    (func $_RNvXNtCsdkdt1aaAg1T_4core3anyReNtB2_3Any7type_idCsdl5sGgnNXvY_3std (;39;) (type 0) (param i32 i32)
      local.get 0
      i32.const 0
      i64.load offset=1048688 align=4
      i64.store offset=8 align=4
      local.get 0
      i32.const 0
      i64.load offset=1048680 align=4
      i64.store align=4
    )
    (func $_RNvXs0_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core3fmt7Display3fmt (;40;) (type 2) (param i32 i32) (result i32)
      block ;; label = @1
        local.get 0
        i32.load
        i32.const -1
        i32.eq
        br_if 0 (;@1;)
        local.get 1
        local.get 0
        i32.load offset=4
        local.get 0
        i32.load offset=8
        call $_RNvMsa_NtCsdkdt1aaAg1T_4core3fmtNtB5_9Formatter9write_str
        return
      end
      local.get 1
      i32.load
      local.get 1
      i32.load offset=4
      local.get 0
      i32.load offset=12
      i32.load
      local.tee 0
      i32.load
      local.get 0
      i32.load offset=4
      call $_RNvNtCsdkdt1aaAg1T_4core3fmt5write
    )
    (func $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload3get (;41;) (type 0) (param i32 i32)
      local.get 0
      i32.const 1049104
      i32.store offset=4
      local.get 0
      local.get 1
      i32.store
    )
    (func $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload6as_str (;42;) (type 0) (param i32 i32)
      local.get 0
      local.get 1
      i64.load align=4
      i64.store
    )
    (func $_RNvXs1_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload8take_box (;43;) (type 0) (param i32 i32)
      (local i32 i32)
      local.get 1
      i32.load offset=4
      local.set 2
      local.get 1
      i32.load
      local.set 3
      call $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2
      block ;; label = @1
        i32.const 8
        i32.const 4
        call $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc
        local.tee 1
        br_if 0 (;@1;)
        i32.const 4
        i32.const 8
        call $_RNvNtCsewWLk9TkM7w_5alloc5alloc18handle_alloc_error
        unreachable
      end
      local.get 1
      local.get 2
      i32.store offset=4
      local.get 1
      local.get 3
      i32.store
      local.get 0
      i32.const 1049104
      i32.store offset=4
      local.get 0
      local.get 1
      i32.store
    )
    (func $_RNvXs2_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB5_16StaticStrPayloadNtNtCsdkdt1aaAg1T_4core3fmt7Display3fmt (;44;) (type 2) (param i32 i32) (result i32)
      local.get 1
      local.get 0
      i32.load
      local.get 0
      i32.load offset=4
      call $_RNvMsa_NtCsdkdt1aaAg1T_4core3fmtNtB5_9Formatter9write_str
    )
    (func $_RNvXsZ_NtCsewWLk9TkM7w_5alloc6stringNtB5_6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write10write_char (;45;) (type 2) (param i32 i32) (result i32)
      (local i32 i32 i32 i32 i32 i32)
      local.get 0
      i32.load offset=8
      local.set 2
      block ;; label = @1
        block ;; label = @2
          local.get 1
          i32.const 128
          i32.ge_u
          br_if 0 (;@2;)
          i32.const 1
          local.set 3
          br 1 (;@1;)
        end
        block ;; label = @2
          local.get 1
          i32.const 2048
          i32.ge_u
          br_if 0 (;@2;)
          i32.const 2
          local.set 3
          br 1 (;@1;)
        end
        i32.const 3
        i32.const 4
        local.get 1
        i32.const 65536
        i32.lt_u
        select
        local.set 3
      end
      block ;; label = @1
        local.get 3
        local.get 0
        i32.load
        local.get 2
        i32.sub
        i32.le_u
        br_if 0 (;@1;)
        local.get 0
        local.get 2
        local.get 3
        i32.const 1
        i32.const 1
        call $_RINvNvMs2_NtCsewWLk9TkM7w_5alloc7raw_vecINtB8_11RawVecInnerpE7reserve21do_reserve_and_handleNtNtBa_5alloc6GlobalECsdl5sGgnNXvY_3std
      end
      local.get 0
      i32.load offset=4
      local.get 2
      i32.add
      local.set 4
      block ;; label = @1
        block ;; label = @2
          local.get 1
          i32.const 128
          i32.lt_u
          br_if 0 (;@2;)
          local.get 1
          i32.const 63
          i32.and
          i32.const -128
          i32.or
          local.set 5
          local.get 1
          i32.const 6
          i32.shr_u
          local.set 6
          block ;; label = @3
            local.get 1
            i32.const 2048
            i32.ge_u
            br_if 0 (;@3;)
            local.get 4
            local.get 5
            i32.store8 offset=1
            local.get 4
            local.get 6
            i32.const 192
            i32.or
            i32.store8
            br 2 (;@1;)
          end
          local.get 1
          i32.const 12
          i32.shr_u
          local.set 7
          local.get 6
          i32.const 63
          i32.and
          i32.const -128
          i32.or
          local.set 6
          block ;; label = @3
            local.get 1
            i32.const 65535
            i32.gt_u
            br_if 0 (;@3;)
            local.get 4
            local.get 5
            i32.store8 offset=2
            local.get 4
            local.get 6
            i32.store8 offset=1
            local.get 4
            local.get 7
            i32.const 224
            i32.or
            i32.store8
            br 2 (;@1;)
          end
          local.get 4
          local.get 5
          i32.store8 offset=3
          local.get 4
          local.get 6
          i32.store8 offset=2
          local.get 4
          local.get 7
          i32.const 63
          i32.and
          i32.const -128
          i32.or
          i32.store8 offset=1
          local.get 4
          local.get 1
          i32.const 18
          i32.shr_u
          i32.const -16
          i32.or
          i32.store8
          br 1 (;@1;)
        end
        local.get 4
        local.get 1
        i32.store8
      end
      local.get 0
      local.get 3
      local.get 2
      i32.add
      i32.store offset=8
      i32.const 0
    )
    (func $_RNvXsZ_NtCsewWLk9TkM7w_5alloc6stringNtB5_6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write9write_str (;46;) (type 1) (param i32 i32 i32) (result i32)
      (local i32)
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 2
            local.get 0
            i32.load
            local.get 0
            i32.load offset=8
            local.tee 3
            i32.sub
            i32.le_u
            br_if 0 (;@3;)
            local.get 0
            local.get 3
            local.get 2
            i32.const 1
            i32.const 1
            call $_RINvNvMs2_NtCsewWLk9TkM7w_5alloc7raw_vecINtB8_11RawVecInnerpE7reserve21do_reserve_and_handleNtNtBa_5alloc6GlobalECsdl5sGgnNXvY_3std
            local.get 0
            i32.load offset=8
            local.set 3
            br 1 (;@2;)
          end
          local.get 2
          i32.eqz
          br_if 1 (;@1;)
        end
        local.get 2
        i32.eqz
        br_if 0 (;@1;)
        local.get 0
        i32.load offset=4
        local.get 3
        i32.add
        local.get 1
        local.get 2
        memory.copy
      end
      local.get 0
      local.get 3
      local.get 2
      i32.add
      i32.store offset=8
      i32.const 0
    )
    (func $_RNvXs_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB4_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload3get (;47;) (type 0) (param i32 i32)
      (local i32 i32 i64)
      global.get $__stack_pointer
      i32.const 32
      i32.sub
      local.tee 2
      global.set $__stack_pointer
      block ;; label = @1
        local.get 1
        i32.load
        i32.const -1
        i32.ne
        br_if 0 (;@1;)
        local.get 1
        i32.load offset=12
        local.set 3
        local.get 2
        i32.const 0
        i32.store offset=28
        local.get 2
        i64.const 4294967296
        i64.store offset=20 align=4
        local.get 2
        i32.const 20
        i32.add
        i32.const 1048600
        local.get 3
        i32.load
        local.tee 3
        i32.load
        local.get 3
        i32.load offset=4
        call $_RNvNtCsdkdt1aaAg1T_4core3fmt5write
        drop
        local.get 2
        local.get 2
        i32.load offset=28
        local.tee 3
        i32.store offset=16
        local.get 2
        local.get 2
        i64.load offset=20 align=4
        local.tee 4
        i64.store offset=8
        local.get 1
        local.get 3
        i32.store offset=8
        local.get 1
        local.get 4
        i64.store align=4
      end
      local.get 0
      i32.const 1049120
      i32.store offset=4
      local.get 0
      local.get 1
      i32.store
      local.get 2
      i32.const 32
      i32.add
      global.set $__stack_pointer
    )
    (func $_RNvXs_NvNtCsdl5sGgnNXvY_3std9panicking13panic_handlerNtB4_19FormatStringPayloadNtNtCsdkdt1aaAg1T_4core5panic12PanicPayload8take_box (;48;) (type 0) (param i32 i32)
      (local i32 i32 i64)
      global.get $__stack_pointer
      i32.const 32
      i32.sub
      local.tee 2
      global.set $__stack_pointer
      block ;; label = @1
        local.get 1
        i32.load
        i32.const -1
        i32.ne
        br_if 0 (;@1;)
        local.get 1
        i32.load offset=12
        local.set 3
        local.get 2
        i32.const 0
        i32.store offset=24
        local.get 2
        i64.const 4294967296
        i64.store offset=16 align=4
        local.get 2
        i32.const 16
        i32.add
        i32.const 1048600
        local.get 3
        i32.load
        local.tee 3
        i32.load
        local.get 3
        i32.load offset=4
        call $_RNvNtCsdkdt1aaAg1T_4core3fmt5write
        drop
        local.get 2
        local.get 2
        i32.load offset=24
        local.tee 3
        i32.store offset=8
        local.get 2
        local.get 2
        i64.load offset=16 align=4
        local.tee 4
        i64.store
        local.get 1
        local.get 3
        i32.store offset=8
        local.get 1
        local.get 4
        i64.store align=4
      end
      local.get 1
      i32.load offset=8
      local.set 3
      local.get 1
      i32.const 0
      i32.store offset=8
      local.get 1
      i64.load align=4
      local.set 4
      local.get 1
      i64.const 4294967296
      i64.store align=4
      local.get 2
      local.get 3
      i32.store offset=24
      local.get 2
      local.get 4
      i64.store offset=16
      call $_RNvCs9wFQrvczXsK_7___rustc35___rust_no_alloc_shim_is_unstable_v2
      block ;; label = @1
        i32.const 12
        i32.const 4
        call $_RNvCs9wFQrvczXsK_7___rustc12___rust_alloc
        local.tee 1
        br_if 0 (;@1;)
        i32.const 4
        i32.const 12
        call $_RNvNtCsewWLk9TkM7w_5alloc5alloc18handle_alloc_error
        unreachable
      end
      local.get 1
      local.get 2
      i32.load offset=24
      i32.store offset=8
      local.get 1
      local.get 2
      i64.load offset=16
      i64.store align=4
      local.get 0
      i32.const 1049120
      i32.store offset=4
      local.get 0
      local.get 1
      i32.store
      local.get 2
      i32.const 32
      i32.add
      global.set $__stack_pointer
    )
    (func $_RNvYINtNvNtCsdl5sGgnNXvY_3std9panicking11begin_panic7PayloadReENtNtCsdkdt1aaAg1T_4core5panic12PanicPayload6as_strB9_ (;49;) (type 0) (param i32 i32)
      local.get 0
      i32.const 0
      i32.store
    )
    (func $_RNvYNtNtCsewWLk9TkM7w_5alloc6string6StringNtNtCsdkdt1aaAg1T_4core3fmt5Write9write_fmtCsdl5sGgnNXvY_3std (;50;) (type 1) (param i32 i32 i32) (result i32)
      local.get 0
      i32.const 1048600
      local.get 1
      local.get 2
      call $_RNvNtCsdkdt1aaAg1T_4core3fmt5write
    )
    (func $_RNvXs_NtCsgis5MWNmFLl_8dlmalloc3sysNtB4_6SystemNtB6_9Allocator5alloc (;51;) (type 5) (param i32 i32 i32)
      (local i32 i32)
      block ;; label = @1
        block ;; label = @2
          local.get 2
          i32.eqz
          br_if 0 (;@2;)
          i32.const 0
          i32.load8_u offset=1049729
          local.set 3
          i32.const 0
          i32.const 1
          i32.store8 offset=1049729
          i32.const 1049744
          local.set 4
          i32.const 1114112
          i32.const 1049744
          i32.le_u
          br_if 0 (;@2;)
          local.get 2
          i32.const 1114112
          i32.const 1049744
          i32.sub
          i32.gt_u
          br_if 0 (;@2;)
          local.get 3
          i32.const 255
          i32.and
          br_if 0 (;@2;)
          i32.const 1114112
          i32.const 1049744
          i32.sub
          local.set 2
          br 1 (;@1;)
        end
        i32.const 0
        local.set 4
        block ;; label = @2
          local.get 2
          i32.const 16
          i32.shr_u
          local.get 2
          i32.const 65535
          i32.and
          i32.const 0
          i32.ne
          i32.add
          local.tee 2
          memory.grow
          local.tee 3
          i32.const -1
          i32.ne
          br_if 0 (;@2;)
          i32.const 0
          local.set 2
          br 1 (;@1;)
        end
        local.get 2
        i32.const 16
        i32.shl
        local.tee 2
        i32.const -16
        i32.add
        local.get 2
        local.get 3
        i32.const 16
        i32.shl
        local.tee 4
        i32.const 0
        local.get 2
        i32.sub
        i32.eq
        select
        local.set 2
      end
      local.get 0
      i32.const 0
      i32.store offset=8
      local.get 0
      local.get 2
      i32.store offset=4
      local.get 0
      local.get 4
      i32.store
    )
    (func $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec12handle_error (;52;) (type 0) (param i32 i32)
      block ;; label = @1
        local.get 0
        i32.eqz
        br_if 0 (;@1;)
        local.get 0
        local.get 1
        call $_RNvNtCsewWLk9TkM7w_5alloc5alloc18handle_alloc_error
        unreachable
      end
      call $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec17capacity_overflow
      unreachable
    )
    (func $_RNvNtCsewWLk9TkM7w_5alloc5alloc18handle_alloc_error (;53;) (type 0) (param i32 i32)
      local.get 1
      local.get 0
      call $_RNvCs9wFQrvczXsK_7___rustc26___rust_alloc_error_handler
      unreachable
    )
    (func $_RNvNtCsewWLk9TkM7w_5alloc7raw_vec17capacity_overflow (;54;) (type 3)
      i32.const 1049192
      i32.const 35
      i32.const 1049212
      call $_RNvNtCsdkdt1aaAg1T_4core9panicking9panic_fmt
      unreachable
    )
    (func $_RNvNtCsdkdt1aaAg1T_4core9panicking5panic (;55;) (type 5) (param i32 i32 i32)
      local.get 0
      local.get 1
      i32.const 1
      i32.shl
      i32.const 1
      i32.or
      local.get 2
      call $_RNvNtCsdkdt1aaAg1T_4core9panicking9panic_fmt
      unreachable
    )
    (func $_RNvNtCsdkdt1aaAg1T_4core9panicking9panic_fmt (;56;) (type 5) (param i32 i32 i32)
      (local i32)
      global.get $__stack_pointer
      i32.const 32
      i32.sub
      local.tee 3
      global.set $__stack_pointer
      local.get 3
      local.get 1
      i32.store offset=16
      local.get 3
      local.get 0
      i32.store offset=12
      local.get 3
      i32.const 1
      i32.store16 offset=28
      local.get 3
      local.get 2
      i32.store offset=24
      local.get 3
      local.get 3
      i32.const 12
      i32.add
      i32.store offset=20
      local.get 3
      i32.const 20
      i32.add
      call $_RNvCs9wFQrvczXsK_7___rustc17rust_begin_unwind
      unreachable
    )
    (func $_RNvXs1i_NtCsdkdt1aaAg1T_4core3fmtReNtB6_7Display3fmtB8_ (;57;) (type 2) (param i32 i32) (result i32)
      local.get 1
      local.get 0
      i32.load
      local.get 0
      i32.load offset=4
      call $_RNvMsa_NtCsdkdt1aaAg1T_4core3fmtNtB5_9Formatter3pad
    )
    (func $_RNvNtCsdkdt1aaAg1T_4core3fmt5write (;58;) (type 6) (param i32 i32 i32 i32) (result i32)
      (local i32 i32 i32 i32 i32 i32 i32 i32)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 4
      global.set $__stack_pointer
      block ;; label = @1
        block ;; label = @2
          block ;; label = @3
            local.get 3
            i32.const 1
            i32.and
            br_if 0 (;@3;)
            local.get 2
            i32.load8_u
            local.tee 5
            br_if 1 (;@2;)
            i32.const 0
            local.set 5
            br 2 (;@1;)
          end
          local.get 0
          local.get 2
          local.get 3
          i32.const 1
          i32.shr_u
          local.get 1
          i32.load offset=12
          call_indirect (type 1)
          local.set 5
          br 1 (;@1;)
        end
        local.get 1
        i32.load offset=12
        local.set 6
        i32.const 0
        local.set 7
        loop ;; label = @2
          local.get 2
          i32.const 1
          i32.add
          local.set 8
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    local.get 5
                    i32.extend8_s
                    i32.const -1
                    i32.gt_s
                    br_if 0 (;@7;)
                    local.get 5
                    i32.const 255
                    i32.and
                    local.tee 9
                    i32.const 128
                    i32.eq
                    br_if 1 (;@6;)
                    local.get 9
                    i32.const 192
                    i32.ne
                    br_if 3 (;@4;)
                    local.get 4
                    local.get 1
                    i32.store offset=4
                    local.get 4
                    local.get 0
                    i32.store
                    local.get 4
                    i64.const 1610612768
                    i64.store offset=8 align=4
                    local.get 3
                    local.get 7
                    i32.const 3
                    i32.shl
                    i32.add
                    local.tee 5
                    i32.load
                    local.get 4
                    local.get 5
                    i32.load offset=4
                    call_indirect (type 2)
                    i32.eqz
                    br_if 2 (;@5;)
                    i32.const 1
                    local.set 5
                    br 6 (;@1;)
                  end
                  block ;; label = @7
                    local.get 0
                    local.get 8
                    local.get 5
                    i32.const 255
                    i32.and
                    local.tee 5
                    local.get 6
                    call_indirect (type 1)
                    br_if 0 (;@7;)
                    local.get 8
                    local.get 5
                    i32.add
                    local.set 2
                    br 4 (;@3;)
                  end
                  i32.const 1
                  local.set 5
                  br 5 (;@1;)
                end
                block ;; label = @6
                  local.get 0
                  local.get 2
                  i32.const 3
                  i32.add
                  local.tee 5
                  local.get 2
                  i32.load16_u offset=1 align=1
                  local.tee 2
                  local.get 6
                  call_indirect (type 1)
                  br_if 0 (;@6;)
                  local.get 5
                  local.get 2
                  i32.add
                  local.set 2
                  br 3 (;@3;)
                end
                i32.const 1
                local.set 5
                br 4 (;@1;)
              end
              local.get 7
              i32.const 1
              i32.add
              local.set 7
              local.get 8
              local.set 2
              br 1 (;@3;)
            end
            i32.const 1610612768
            local.set 10
            block ;; label = @4
              local.get 5
              i32.const 1
              i32.and
              i32.eqz
              br_if 0 (;@4;)
              local.get 2
              i32.const 5
              i32.add
              local.set 8
              local.get 2
              i32.load offset=1 align=1
              local.set 10
            end
            i32.const 0
            local.set 9
            block ;; label = @4
              block ;; label = @5
                local.get 5
                i32.const 2
                i32.and
                br_if 0 (;@5;)
                i32.const 0
                local.set 11
                local.get 8
                local.set 2
                br 1 (;@4;)
              end
              local.get 8
              i32.const 2
              i32.add
              local.set 2
              local.get 8
              i32.load16_u align=1
              local.set 11
            end
            block ;; label = @4
              block ;; label = @5
                local.get 5
                i32.const 4
                i32.and
                br_if 0 (;@5;)
                local.get 2
                local.set 8
                br 1 (;@4;)
              end
              local.get 2
              i32.const 2
              i32.add
              local.set 8
              local.get 2
              i32.load16_u align=1
              local.set 9
            end
            block ;; label = @4
              block ;; label = @5
                local.get 5
                i32.const 8
                i32.and
                br_if 0 (;@5;)
                local.get 8
                local.set 2
                br 1 (;@4;)
              end
              local.get 8
              i32.const 2
              i32.add
              local.set 2
              local.get 8
              i32.load16_u align=1
              local.set 7
            end
            block ;; label = @4
              local.get 5
              i32.const 16
              i32.and
              i32.eqz
              br_if 0 (;@4;)
              local.get 3
              local.get 11
              i32.const 65535
              i32.and
              i32.const 3
              i32.shl
              i32.add
              i32.load16_u offset=4
              local.set 11
            end
            block ;; label = @4
              local.get 5
              i32.const 32
              i32.and
              i32.eqz
              br_if 0 (;@4;)
              local.get 3
              local.get 9
              i32.const 65535
              i32.and
              i32.const 3
              i32.shl
              i32.add
              i32.load16_u offset=4
              local.set 9
            end
            local.get 4
            local.get 9
            i32.store16 offset=14
            local.get 4
            local.get 11
            i32.store16 offset=12
            local.get 4
            local.get 10
            i32.store offset=8
            local.get 4
            local.get 1
            i32.store offset=4
            local.get 4
            local.get 0
            i32.store
            block ;; label = @4
              local.get 3
              local.get 7
              i32.const 3
              i32.shl
              i32.add
              local.tee 5
              i32.load
              local.get 4
              local.get 5
              i32.load offset=4
              call_indirect (type 2)
              i32.eqz
              br_if 0 (;@4;)
              i32.const 1
              local.set 5
              br 3 (;@1;)
            end
            local.get 7
            i32.const 1
            i32.add
            local.set 7
          end
          local.get 2
          i32.load8_u
          local.tee 5
          br_if 0 (;@2;)
        end
        i32.const 0
        local.set 5
      end
      local.get 4
      i32.const 16
      i32.add
      global.set $__stack_pointer
      local.get 5
    )
    (func $_RNvNtNtCsdkdt1aaAg1T_4core3str5count14do_count_chars (;59;) (type 2) (param i32 i32) (result i32)
      (local i32 i32 i32 i32 i32 i32 i32 i32)
      block ;; label = @1
        block ;; label = @2
          local.get 1
          local.get 0
          i32.const 3
          i32.add
          i32.const -4
          i32.and
          local.tee 2
          local.get 0
          i32.sub
          local.tee 3
          i32.lt_u
          br_if 0 (;@2;)
          local.get 1
          local.get 3
          i32.sub
          local.tee 4
          i32.const 2
          i32.shr_u
          local.tee 5
          i32.eqz
          br_if 0 (;@2;)
          local.get 4
          i32.const 3
          i32.and
          local.set 6
          i32.const 0
          local.set 7
          i32.const 0
          local.set 1
          block ;; label = @3
            local.get 2
            local.get 0
            i32.eq
            br_if 0 (;@3;)
            i32.const 0
            local.set 8
            i32.const 0
            local.set 1
            block ;; label = @4
              local.get 0
              local.get 2
              i32.sub
              local.tee 9
              i32.const -4
              i32.gt_u
              br_if 0 (;@4;)
              i32.const 0
              local.set 8
              i32.const 0
              local.set 1
              loop ;; label = @5
                local.get 1
                local.get 0
                local.get 8
                i32.add
                local.tee 2
                i32.load8_s
                i32.const -65
                i32.gt_s
                i32.add
                local.get 2
                i32.const 1
                i32.add
                i32.load8_s
                i32.const -65
                i32.gt_s
                i32.add
                local.get 2
                i32.const 2
                i32.add
                i32.load8_s
                i32.const -65
                i32.gt_s
                i32.add
                local.get 2
                i32.const 3
                i32.add
                i32.load8_s
                i32.const -65
                i32.gt_s
                i32.add
                local.set 1
                local.get 8
                i32.const 4
                i32.add
                local.tee 8
                br_if 0 (;@5;)
              end
            end
            local.get 0
            local.get 8
            i32.add
            local.set 2
            loop ;; label = @4
              local.get 1
              local.get 2
              i32.load8_s
              i32.const -65
              i32.gt_s
              i32.add
              local.set 1
              local.get 2
              i32.const 1
              i32.add
              local.set 2
              local.get 9
              i32.const 1
              i32.add
              local.tee 9
              br_if 0 (;@4;)
            end
          end
          local.get 0
          local.get 3
          i32.add
          local.set 9
          block ;; label = @3
            local.get 6
            i32.eqz
            br_if 0 (;@3;)
            local.get 9
            local.get 4
            i32.const 2147483644
            i32.and
            i32.add
            local.tee 2
            i32.load8_s
            i32.const -65
            i32.gt_s
            local.set 7
            local.get 6
            i32.const 1
            i32.eq
            br_if 0 (;@3;)
            local.get 7
            local.get 2
            i32.load8_s offset=1
            i32.const -65
            i32.gt_s
            i32.add
            local.set 7
            local.get 6
            i32.const 2
            i32.eq
            br_if 0 (;@3;)
            local.get 7
            local.get 2
            i32.load8_s offset=2
            i32.const -65
            i32.gt_s
            i32.add
            local.set 7
          end
          local.get 7
          local.get 1
          i32.add
          local.set 8
          loop ;; label = @3
            local.get 9
            local.set 3
            local.get 5
            i32.eqz
            br_if 2 (;@1;)
            local.get 5
            i32.const 192
            local.get 5
            i32.const 192
            i32.lt_u
            select
            local.tee 7
            i32.const 3
            i32.and
            local.set 6
            block ;; label = @4
              block ;; label = @5
                local.get 7
                i32.const 2
                i32.shl
                local.tee 4
                i32.const 1008
                i32.and
                local.tee 1
                br_if 0 (;@5;)
                i32.const 0
                local.set 2
                br 1 (;@4;)
              end
              local.get 3
              local.get 1
              i32.add
              local.set 0
              i32.const 0
              local.set 2
              local.get 3
              local.set 1
              loop ;; label = @5
                local.get 1
                i32.const 12
                i32.add
                i32.load
                local.tee 9
                i32.const -1
                i32.xor
                i32.const 7
                i32.shr_u
                local.get 9
                i32.const 6
                i32.shr_u
                i32.or
                i32.const 16843009
                i32.and
                local.get 1
                i32.const 8
                i32.add
                i32.load
                local.tee 9
                i32.const -1
                i32.xor
                i32.const 7
                i32.shr_u
                local.get 9
                i32.const 6
                i32.shr_u
                i32.or
                i32.const 16843009
                i32.and
                local.get 1
                i32.const 4
                i32.add
                i32.load
                local.tee 9
                i32.const -1
                i32.xor
                i32.const 7
                i32.shr_u
                local.get 9
                i32.const 6
                i32.shr_u
                i32.or
                i32.const 16843009
                i32.and
                local.get 1
                i32.load
                local.tee 9
                i32.const -1
                i32.xor
                i32.const 7
                i32.shr_u
                local.get 9
                i32.const 6
                i32.shr_u
                i32.or
                i32.const 16843009
                i32.and
                local.get 2
                i32.add
                i32.add
                i32.add
                i32.add
                local.set 2
                local.get 1
                i32.const 16
                i32.add
                local.tee 1
                local.get 0
                i32.ne
                br_if 0 (;@5;)
              end
            end
            local.get 5
            local.get 7
            i32.sub
            local.set 5
            local.get 3
            local.get 4
            i32.add
            local.set 9
            local.get 2
            i32.const 8
            i32.shr_u
            i32.const 16711935
            i32.and
            local.get 2
            i32.const 16711935
            i32.and
            i32.add
            i32.const 65537
            i32.mul
            i32.const 16
            i32.shr_u
            local.get 8
            i32.add
            local.set 8
            local.get 6
            i32.eqz
            br_if 0 (;@3;)
          end
          local.get 3
          local.get 7
          i32.const 252
          i32.and
          i32.const 2
          i32.shl
          i32.add
          local.tee 2
          i32.load
          local.tee 1
          i32.const -1
          i32.xor
          i32.const 7
          i32.shr_u
          local.get 1
          i32.const 6
          i32.shr_u
          i32.or
          i32.const 16843009
          i32.and
          local.set 1
          block ;; label = @3
            local.get 6
            i32.const 1
            i32.eq
            br_if 0 (;@3;)
            local.get 2
            i32.load offset=4
            local.tee 9
            i32.const -1
            i32.xor
            i32.const 7
            i32.shr_u
            local.get 9
            i32.const 6
            i32.shr_u
            i32.or
            i32.const 16843009
            i32.and
            local.get 1
            i32.add
            local.set 1
            local.get 6
            i32.const 2
            i32.eq
            br_if 0 (;@3;)
            local.get 2
            i32.load offset=8
            local.tee 2
            i32.const -1
            i32.xor
            i32.const 7
            i32.shr_u
            local.get 2
            i32.const 6
            i32.shr_u
            i32.or
            i32.const 16843009
            i32.and
            local.get 1
            i32.add
            local.set 1
          end
          local.get 1
          i32.const 8
          i32.shr_u
          i32.const 459007
          i32.and
          local.get 1
          i32.const 16711935
          i32.and
          i32.add
          i32.const 65537
          i32.mul
          i32.const 16
          i32.shr_u
          local.get 8
          i32.add
          local.set 8
          br 1 (;@1;)
        end
        block ;; label = @2
          local.get 1
          br_if 0 (;@2;)
          i32.const 0
          return
        end
        local.get 1
        i32.const 3
        i32.and
        local.set 2
        i32.const 0
        local.set 9
        i32.const 0
        local.set 8
        block ;; label = @2
          local.get 1
          i32.const 4
          i32.lt_u
          br_if 0 (;@2;)
          local.get 1
          i32.const -4
          i32.and
          local.set 5
          i32.const 0
          local.set 8
          i32.const 0
          local.set 9
          loop ;; label = @3
            local.get 8
            local.get 0
            local.get 9
            i32.add
            local.tee 1
            i32.load8_s
            i32.const -65
            i32.gt_s
            i32.add
            local.get 1
            i32.const 1
            i32.add
            i32.load8_s
            i32.const -65
            i32.gt_s
            i32.add
            local.get 1
            i32.const 2
            i32.add
            i32.load8_s
            i32.const -65
            i32.gt_s
            i32.add
            local.get 1
            i32.const 3
            i32.add
            i32.load8_s
            i32.const -65
            i32.gt_s
            i32.add
            local.set 8
            local.get 5
            local.get 9
            i32.const 4
            i32.add
            local.tee 9
            i32.ne
            br_if 0 (;@3;)
          end
          local.get 2
          i32.eqz
          br_if 1 (;@1;)
        end
        local.get 0
        local.get 9
        i32.add
        local.set 1
        loop ;; label = @2
          local.get 8
          local.get 1
          i32.load8_s
          i32.const -65
          i32.gt_s
          i32.add
          local.set 8
          local.get 1
          i32.const 1
          i32.add
          local.set 1
          local.get 2
          i32.const -1
          i32.add
          local.tee 2
          br_if 0 (;@2;)
        end
      end
      local.get 8
    )
    (func $_RNvMsa_NtCsdkdt1aaAg1T_4core3fmtNtB5_9Formatter3pad (;60;) (type 1) (param i32 i32 i32) (result i32)
      (local i32 i32 i32 i32 i32 i32 i32)
      block ;; label = @1
        block ;; label = @2
          local.get 0
          i32.load offset=8
          local.tee 3
          i32.const 402653184
          i32.and
          i32.eqz
          br_if 0 (;@2;)
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                block ;; label = @6
                  block ;; label = @7
                    local.get 3
                    i32.const 268435456
                    i32.and
                    i32.eqz
                    br_if 0 (;@7;)
                    local.get 0
                    i32.load16_u offset=14
                    local.tee 4
                    br_if 1 (;@6;)
                    i32.const 0
                    local.set 2
                    br 2 (;@5;)
                  end
                  block ;; label = @7
                    local.get 2
                    i32.const 16
                    i32.lt_u
                    br_if 0 (;@7;)
                    local.get 1
                    local.get 2
                    call $_RNvNtNtCsdkdt1aaAg1T_4core3str5count14do_count_chars
                    local.set 5
                    br 4 (;@3;)
                  end
                  block ;; label = @7
                    local.get 2
                    br_if 0 (;@7;)
                    i32.const 0
                    local.set 5
                    br 4 (;@3;)
                  end
                  local.get 2
                  i32.const 3
                  i32.and
                  local.set 6
                  i32.const 0
                  local.set 7
                  i32.const 0
                  local.set 5
                  block ;; label = @7
                    local.get 2
                    i32.const 4
                    i32.lt_u
                    br_if 0 (;@7;)
                    local.get 2
                    i32.const 12
                    i32.and
                    local.set 4
                    i32.const 0
                    local.set 5
                    i32.const 0
                    local.set 7
                    loop ;; label = @8
                      local.get 5
                      local.get 1
                      local.get 7
                      i32.add
                      local.tee 8
                      i32.load8_s
                      i32.const -65
                      i32.gt_s
                      i32.add
                      local.get 8
                      i32.const 1
                      i32.add
                      i32.load8_s
                      i32.const -65
                      i32.gt_s
                      i32.add
                      local.get 8
                      i32.const 2
                      i32.add
                      i32.load8_s
                      i32.const -65
                      i32.gt_s
                      i32.add
                      local.get 8
                      i32.const 3
                      i32.add
                      i32.load8_s
                      i32.const -65
                      i32.gt_s
                      i32.add
                      local.set 5
                      local.get 4
                      local.get 7
                      i32.const 4
                      i32.add
                      local.tee 7
                      i32.ne
                      br_if 0 (;@8;)
                    end
                    local.get 6
                    i32.eqz
                    br_if 4 (;@3;)
                  end
                  local.get 1
                  local.get 7
                  i32.add
                  local.set 8
                  loop ;; label = @7
                    local.get 5
                    local.get 8
                    i32.load8_s
                    i32.const -65
                    i32.gt_s
                    i32.add
                    local.set 5
                    local.get 8
                    i32.const 1
                    i32.add
                    local.set 8
                    local.get 6
                    i32.const -1
                    i32.add
                    local.tee 6
                    br_if 0 (;@7;)
                    br 4 (;@3;)
                  end
                end
                local.get 1
                local.get 2
                i32.add
                local.set 7
                i32.const 0
                local.set 2
                local.get 1
                local.set 8
                local.get 4
                local.set 6
                loop ;; label = @6
                  local.get 8
                  local.tee 5
                  local.get 7
                  i32.eq
                  br_if 2 (;@4;)
                  block ;; label = @7
                    block ;; label = @8
                      local.get 5
                      i32.load8_s
                      local.tee 8
                      i32.const -1
                      i32.le_s
                      br_if 0 (;@8;)
                      local.get 5
                      i32.const 1
                      i32.add
                      local.set 8
                      br 1 (;@7;)
                    end
                    block ;; label = @8
                      local.get 8
                      i32.const -32
                      i32.ge_u
                      br_if 0 (;@8;)
                      local.get 5
                      i32.const 2
                      i32.add
                      local.set 8
                      br 1 (;@7;)
                    end
                    local.get 5
                    i32.const 4
                    i32.const 3
                    local.get 8
                    i32.const -17
                    i32.gt_u
                    select
                    i32.add
                    local.set 8
                  end
                  local.get 8
                  local.get 5
                  i32.sub
                  local.get 2
                  i32.add
                  local.set 2
                  local.get 6
                  i32.const -1
                  i32.add
                  local.tee 6
                  br_if 0 (;@6;)
                end
              end
              i32.const 0
              local.set 6
            end
            local.get 4
            local.get 6
            i32.sub
            local.set 5
          end
          local.get 5
          local.get 0
          i32.load16_u offset=12
          local.tee 8
          i32.ge_u
          br_if 0 (;@2;)
          local.get 8
          local.get 5
          i32.sub
          local.set 9
          i32.const 0
          local.set 5
          i32.const 0
          local.set 4
          block ;; label = @3
            block ;; label = @4
              block ;; label = @5
                local.get 3
                i32.const 29
                i32.shr_u
                i32.const 3
                i32.and
                br_table 2 (;@3;) 0 (;@5;) 1 (;@4;) 2 (;@3;) 2 (;@3;)
              end
              local.get 9
              local.set 4
              br 1 (;@3;)
            end
            local.get 9
            i32.const 65534
            i32.and
            i32.const 1
            i32.shr_u
            local.set 4
          end
          local.get 3
          i32.const 2097151
          i32.and
          local.set 7
          local.get 0
          i32.load offset=4
          local.set 6
          local.get 0
          i32.load
          local.set 0
          block ;; label = @3
            loop ;; label = @4
              local.get 5
              i32.const 65535
              i32.and
              local.get 4
              i32.const 65535
              i32.and
              i32.ge_u
              br_if 1 (;@3;)
              i32.const 1
              local.set 8
              local.get 5
              i32.const 1
              i32.add
              local.set 5
              local.get 0
              local.get 7
              local.get 6
              i32.load offset=16
              call_indirect (type 2)
              br_if 3 (;@1;)
              br 0 (;@4;)
            end
          end
          i32.const 1
          local.set 8
          local.get 0
          local.get 1
          local.get 2
          local.get 6
          i32.load offset=12
          call_indirect (type 1)
          br_if 1 (;@1;)
          i32.const 0
          local.set 5
          local.get 9
          local.get 4
          i32.sub
          i32.const 65535
          i32.and
          local.set 2
          loop ;; label = @3
            local.get 5
            i32.const 65535
            i32.and
            local.tee 4
            local.get 2
            i32.lt_u
            local.set 8
            local.get 4
            local.get 2
            i32.ge_u
            br_if 2 (;@1;)
            local.get 5
            i32.const 1
            i32.add
            local.set 5
            local.get 0
            local.get 7
            local.get 6
            i32.load offset=16
            call_indirect (type 2)
            br_if 2 (;@1;)
            br 0 (;@3;)
          end
        end
        local.get 0
        i32.load
        local.get 1
        local.get 2
        local.get 0
        i32.load offset=4
        i32.load offset=12
        call_indirect (type 1)
        local.set 8
      end
      local.get 8
    )
    (func $_RNvMsa_NtCsdkdt1aaAg1T_4core3fmtNtB5_9Formatter9write_str (;61;) (type 1) (param i32 i32 i32) (result i32)
      local.get 0
      i32.load
      local.get 1
      local.get 2
      local.get 0
      i32.load offset=4
      i32.load offset=12
      call_indirect (type 1)
    )
    (func $_RNvNtCsdkdt1aaAg1T_4core6option13expect_failed (;62;) (type 5) (param i32 i32 i32)
      (local i32)
      global.get $__stack_pointer
      i32.const 16
      i32.sub
      local.tee 3
      global.set $__stack_pointer
      local.get 3
      local.get 1
      i32.store offset=4
      local.get 3
      local.get 0
      i32.store
      local.get 3
      i32.const 19
      i64.extend_i32_u
      i64.const 32
      i64.shl
      local.get 3
      i64.extend_i32_u
      i64.or
      i64.store offset=8
      i32.const 1048712
      local.get 3
      i32.const 8
      i32.add
      local.get 2
      call $_RNvNtCsdkdt1aaAg1T_4core9panicking9panic_fmt
      unreachable
    )
    (data $.rodata (;0;) (i32.const 1048576) "\01\00\00\00fixture failure\00\02\00\00\00\04\00\00\00\0c\00\00\00\04\00\00\00\05\00\00\00\06\00\00\00\07\00\00\00\00\00\00\00\08\00\00\00\04\00\00\00\08\00\00\00\09\00\00\00\0a\00\00\00\0b\00\00\00\0c\00\00\00\10\00\00\00\04\00\00\00\0d\00\00\00\0e\00\00\00\0f\00\00\00\10\00\00\00\5c\f6\e9_\dc\02\f6\b9\f1\c1pl\f2a\c1$\da\07\8cIxeL\d3\c2}\8fM\96\9f&\cf\c0\00/rustc/8bab26f4f68e0e26f0bb7960be334d5b520ea452/library/std/src/sys/sync/rwlock/no_threads.rs\00/rustc/8bab26f4f68e0e26f0bb7960be334d5b520ea452/library/alloc/src/raw_vec/mod.rs\00/rust/deps/dlmalloc-0.2.13/src/dlmalloc.rs\00assertion failed: psize >= size + min_overhead\00\009\01\10\00*\00\00\00\b1\04\00\00\09\00\00\00assertion failed: psize <= size + max_overhead\00\009\01\10\00*\00\00\00\b7\04\00\00\0d\00\00\00rwlock overflowed read locks\8a\00\10\00]\00\00\00\15\00\00\00,\00\00\00\00\00\00\00\08\00\00\00\04\00\00\00\11\00\00\00\04\00\00\00\0c\00\00\00\04\00\00\00\12\00\00\00rwlock has not been locked for reading\00\00\8a\00\10\00]\00\00\00>\00\00\00\09\00\00\00capacity overflow\00\00\00\e8\00\10\00P\00\00\00\1c\00\00\00\05\00\00\00")
    (@producers
      (language "Rust" "")
      (processed-by "rustc" "1.97.1 (8bab26f4f 2026-07-14)")
      (processed-by "wit-component" "0.247.0")
      (processed-by "wit-bindgen-rust" "0.57.1")
    )
    (@custom "target_features" (after data) "\08+\0bbulk-memory+\0fbulk-memory-opt+\16call-indirect-overlong+\0amultivalue+\0fmutable-globals+\13nontrapping-fptoint+\0freference-types+\08sign-ext")
  )
  (type (;0;) (enum "invalid-input" "failed"))
  (type (;1;) (record (field "code" 0) (field "message" string)))
  (core instance $main (;0;) (instantiate $main))
  (alias core export $main "memory" (core memory $memory (;0;)))
  (type (;2;) (list u8))
  (type (;3;) (result 2 (error 1)))
  (type (;4;) (func (param "input" 2) (result 3)))
  (alias core export $main "hologram:application/guest@1.0.0#run" (core func $hologram:application/guest@1.0.0#run (;0;)))
  (alias core export $main "cabi_realloc" (core func $cabi_realloc (;1;)))
  (alias core export $main "cabi_post_hologram:application/guest@1.0.0#run" (core func $cabi_post_hologram:application/guest@1.0.0#run (;2;)))
  (func $run (;0;) (type 4) (canon lift (core func $hologram:application/guest@1.0.0#run) (memory $memory) (realloc $cabi_realloc) string-encoding=utf8 (post-return $cabi_post_hologram:application/guest@1.0.0#run)))
  (component $hologram:application/guest@1.0.0-shim-component (;0;)
    (type (;0;) (list u8))
    (type (;1;) (enum "invalid-input" "failed"))
    (import "import-type-error-code" (type (;2;) (eq 1)))
    (type (;3;) (record (field "code" 2) (field "message" string)))
    (import "import-type-guest-error" (type (;4;) (eq 3)))
    (type (;5;) (result 0 (error 4)))
    (type (;6;) (func (param "input" 0) (result 5)))
    (import "import-func-run" (func (;0;) (type 6)))
    (type (;7;) (enum "invalid-input" "failed"))
    (export (;8;) "error-code" (type 7))
    (type (;9;) (record (field "code" 8) (field "message" string)))
    (export (;10;) "guest-error" (type 9))
    (type (;11;) (list u8))
    (type (;12;) (result 11 (error 10)))
    (type (;13;) (func (param "input" 11) (result 12)))
    (export (;1;) "run" (func 0) (func (type 13)))
  )
  (instance $hologram:application/guest@1.0.0-shim-instance (;0;) (instantiate $hologram:application/guest@1.0.0-shim-component
      (with "import-func-run" (func $run))
      (with "import-type-error-code" (type 0))
      (with "import-type-guest-error" (type 1))
    )
  )
  (export $hologram:application/guest@1.0.0 (;1;) "hologram:application/guest@1.0.0" (instance $hologram:application/guest@1.0.0-shim-instance))
  (@producers
    (processed-by "wit-component" "0.247.0")
  )
)
