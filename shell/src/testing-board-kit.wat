;; A Wasm kit that uses another (wasm_kit.rs): acme:board/score's score()
;; answers acme:crm's count-leads("web") + 1 (grant leads-read).
(component
  (import "acme:crm/leads@1.0.0" (instance $leads
    (export "count-leads" (func (param "source" string) (result u32)))))
  (alias export $leads "count-leads" (func $count))
  (core module $storage
    (memory (export "memory") 1)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) i32.const 64))
  (core instance $storage (instantiate $storage))
  (alias core export $storage "memory" (core memory $memory))
  (core func $count (canon lower (func $count) (memory $memory)))
  (core module $kit
    (import "storage" "memory" (memory 1))
    (import "host" "count" (func $count (param i32 i32) (result i32)))
    (data (i32.const 0) "web")
    (func (export "score") (result i32)
      i32.const 0 i32.const 3 call $count
      i32.const 1 i32.add))
  (core instance $kit (instantiate $kit
    (with "storage" (instance $storage))
    (with "host" (instance (export "count" (func $count))))))
  (func $score (result u32) (canon lift (core func $kit "score")))
  (instance $score (export "score" (func $score)))
  (export "acme:board/score@1.0.0" (instance $score)))
