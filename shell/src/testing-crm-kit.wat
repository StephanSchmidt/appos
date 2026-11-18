;; A Wasm kit for the deploy tests (wasm_kit.rs): acme:crm's leads, with
;; count-leads answering 7 and add-lead doing nothing, and a page at /
;; that shows "leads here". The tests change the answer, or make it trap.
(component
  (type $pairs (list (tuple string string)))
  (type $request (record
    (field "method" string) (field "path" string)
    (field "params" $pairs) (field "query" $pairs) (field "form" $pairs)
    (field "base" string) (field "origin" string) (field "htmx" bool) (field "now" string)))
  (import "appos:framework/registry@0.1.0" (instance $registry
    (export "add-route" (func (param "method" string) (param "path" string) (param "handler" string)))))
  (import "appos:framework/page@0.1.0" (instance $page
    (export "set-html" (func (param "name" string) (param "html" string)))))
  (alias export $registry "add-route" (func $route))
  (alias export $page "set-html" (func $html))
  (core module $storage
    (memory (export "memory") 2)
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
  (core func $route (canon lower (func $route) (memory $memory)))
  (core func $html (canon lower (func $html) (memory $memory)))
  (core module $kit
    (import "storage" "memory" (memory 2))
    (import "host" "route" (func $route (param i32 i32 i32 i32 i32 i32)))
    (import "host" "html" (func $html (param i32 i32 i32 i32)))
    (data (i32.const 0) "GET/index")
    (data (i32.const 16) "pageleads here")
    (func (export "init")
      i32.const 0 i32.const 3 i32.const 3 i32.const 1 i32.const 4 i32.const 5 call $route)
    (func (export "start") (param i32)
      i32.const 16 i32.const 4 i32.const 20 i32.const 10 call $html)
    (func (export "count") (param i32 i32) (result i32)
      i32.const 7)
    (func (export "add") (param i32 i32)))
  (core instance $kit (instantiate $kit
    (with "storage" (instance $storage))
    (with "host" (instance (export "route" (func $route)) (export "html" (func $html))))))
  (func $init (canon lift (core func $kit "init")))
  (func $start (param "handler" string) (param "req" $request)
    (canon lift (core func $kit "start") (memory $memory) (realloc $realloc)))
  (instance $handler
    (export "request" (type $request))
    (export "init" (func $init)) (export "start" (func $start)))
  (export "appos:framework/handler@0.1.0" (instance $handler))
  (func $count (param "source" string) (result u32)
    (canon lift (core func $kit "count") (memory $memory) (realloc $realloc)))
  (func $add (param "source" string)
    (canon lift (core func $kit "add") (memory $memory) (realloc $realloc)))
  (instance $leads (export "count-leads" (func $count)) (export "add-lead" (func $add)))
  (export "acme:crm/leads@1.0.0" (instance $leads)))
