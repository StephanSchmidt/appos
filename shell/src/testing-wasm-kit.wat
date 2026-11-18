;; A kit in Wasm, for the Wasm kit tests (wasm_kit.rs): it offers acme:crm's
;; leads, as the Rust test kit of kit.rs does, and calls the Rust test kit
;; acme:bonus. count-leads answers 5 + bonus(), and spins until its fuel
;; runs out when bonus() is 0; the command add-lead(source) calls
;; note(source).
(component
  (import "acme:bonus/b@1.0.0" (instance $bonus
    (export "bonus" (func (result u32)))
    (export "note" (func (param "text" string)))))
  (alias export $bonus "bonus" (func $bonus))
  (alias export $bonus "note" (func $note))
  (core module $storage
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 1024))
    (func (export "realloc") (param i32 i32) (param $align i32) (param $size i32) (result i32)
      (local $ptr i32)
      global.get $heap
      local.get $align i32.const 1 i32.sub i32.add
      i32.const 0 local.get $align i32.sub i32.and
      local.tee $ptr local.get $size i32.add global.set $heap
      local.get $ptr))
  (core instance $storage (instantiate $storage))
  (alias core export $storage "memory" (core memory $memory))
  (alias core export $storage "realloc" (core func $realloc))
  (core func $bonus (canon lower (func $bonus)))
  (core func $note (canon lower (func $note) (memory $memory)))
  (core module $kit
    (import "host" "bonus" (func $bonus (result i32)))
    (import "host" "note" (func $note (param i32 i32)))
    (func (export "count") (param i32 i32) (result i32)
      (local $b i32)
      call $bonus
      local.tee $b
      i32.eqz
      if
        (loop $spin (br $spin))
      end
      local.get $b
      i32.const 5
      i32.add)
    (func (export "add") (param $ptr i32) (param $len i32)
      local.get $ptr local.get $len call $note))
  (core instance $kit (instantiate $kit
    (with "host" (instance (export "bonus" (func $bonus)) (export "note" (func $note))))))
  (func $count (param "source" string) (result u32)
    (canon lift (core func $kit "count") (memory $memory) (realloc $realloc)))
  (func $add (param "source" string)
    (canon lift (core func $kit "add") (memory $memory) (realloc $realloc)))
  (instance $leads (export "count-leads" (func $count)) (export "add-lead" (func $add)))
  (export "acme:crm/leads@1.0.0" (instance $leads)))
