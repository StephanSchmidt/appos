;; A test app that uses kv: GET / reads "k" (kv-read), POST / writes
;; "k" = "v" (kv-write); both then show "ok". For the policy tests.
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
  (import "appos:framework/kv@0.1.0" (instance $kv
    (export "get" (func (param "key" string) (result (option string))))
    (export "set" (func (param "key" string) (param "value" string)))))
  (alias export $registry "add-route" (func $route))
  (alias export $page "set-html" (func $html))
  (alias export $kv "get" (func $get))
  (alias export $kv "set" (func $set))
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
  (core func $get (canon lower (func $get) (memory $memory) (realloc $realloc)))
  (core func $set (canon lower (func $set) (memory $memory)))
  (core module $app
    (import "storage" "memory" (memory 2))
    (import "host" "route" (func $route (param i32 i32 i32 i32 i32 i32)))
    (import "host" "html" (func $html (param i32 i32 i32 i32)))
    (import "host" "get" (func $get (param i32 i32 i32)))
    (import "host" "set" (func $set (param i32 i32 i32 i32)))
    ;; "GET" 0, "/" 3, "read" 4, "POST" 8, "write" 12
    (data (i32.const 0) "GET/readPOSTwrite")
    ;; "page" 32, "ok" 36, "k" 38, "v" 39
    (data (i32.const 32) "pageokkv")
    (func (export "init")
      i32.const 0 i32.const 3 i32.const 3 i32.const 1 i32.const 4 i32.const 4 call $route
      i32.const 8 i32.const 4 i32.const 3 i32.const 1 i32.const 12 i32.const 5 call $route)
    ;; The arguments are in memory: the handler's name first, its length
    ;; at +4; "write" is the one of length 5.
    (func (export "start") (param $args i32)
      local.get $args i32.load offset=4 i32.const 5 i32.eq
      if
        i32.const 38 i32.const 1 i32.const 39 i32.const 1 call $set
      else
        i32.const 38 i32.const 1 i32.const 64 call $get
      end
      i32.const 32 i32.const 4 i32.const 36 i32.const 2 call $html))
  (core instance $app (instantiate $app
    (with "storage" (instance $storage))
    (with "host" (instance
      (export "route" (func $route)) (export "html" (func $html))
      (export "get" (func $get)) (export "set" (func $set))))))
  (func $init (canon lift (core func $app "init")))
  (func $start (param "handler" string) (param "req" $request)
    (canon lift (core func $app "start") (memory $memory) (realloc $realloc)))
  (instance $handler
    (export "request" (type $request))
    (export "init" (func $init)) (export "start" (func $start)))
  (export "appos:framework/handler@0.1.0" (instance $handler)))
