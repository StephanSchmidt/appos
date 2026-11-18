;; A test app that sends commands to a platform's kit: handler "send"
;; sends acme:crm's add-lead("web") and add-lead("ads") (grant
;; leads-write) and shows "sent"; handler "crash" sends add-lead("web"),
;; then crashes. For the outbox tests (kit.rs).
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
  (import "acme:crm/leads@1.0.0" (instance $leads
    (export "add-lead" (func (param "source" string)))))
  (alias export $registry "add-route" (func $route))
  (alias export $page "set-html" (func $html))
  (alias export $leads "add-lead" (func $add))
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
  (core func $add (canon lower (func $add) (memory $memory)))
  (core module $app
    (import "storage" "memory" (memory 2))
    (import "host" "route" (func $route (param i32 i32 i32 i32 i32 i32)))
    (import "host" "html" (func $html (param i32 i32 i32 i32)))
    (import "host" "add" (func $add (param i32 i32)))
    ;; "GET" 0, "/" 3, "send" 4, "/crash" 8, "crash" 14
    (data (i32.const 0) "GET/send/crashcrash")
    ;; "page" 32, "sent" 36, "web" 40, "ads" 43
    (data (i32.const 32) "pagesentwebads")
    (func (export "init")
      i32.const 0 i32.const 3 i32.const 3 i32.const 1 i32.const 4 i32.const 4 call $route
      i32.const 0 i32.const 3 i32.const 8 i32.const 6 i32.const 14 i32.const 5 call $route)
    ;; The handler's name is the first parameter: its length tells them apart.
    (func (export "start") (param $args i32)
      i32.const 40 i32.const 3 call $add
      local.get $args i32.load offset=4
      i32.const 5 i32.eq
      if unreachable end
      i32.const 43 i32.const 3 call $add
      i32.const 32 i32.const 4 i32.const 36 i32.const 4 call $html))
  (core instance $app (instantiate $app
    (with "storage" (instance $storage))
    (with "host" (instance
      (export "route" (func $route)) (export "html" (func $html)) (export "add" (func $add))))))
  (func $init (canon lift (core func $app "init")))
  (func $start (param "handler" string) (param "req" $request)
    (canon lift (core func $app "start") (memory $memory) (realloc $realloc)))
  (instance $handler
    (export "request" (type $request))
    (export "init" (func $init)) (export "start" (func $start)))
  (export "appos:framework/handler@0.1.0" (instance $handler)))
