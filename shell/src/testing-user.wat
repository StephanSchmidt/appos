;; A test app that shows who is signed in: GET / (handler "who") asks
;; users::current (grant user-read) and shows the email, or "nobody". For
;; the user tests (kit.rs).
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
  (import "appos:framework/users@0.1.0" (instance $users
    (type $record (record
      (field "id" string) (field "email" string) (field "name" string) (field "groups" (list string))))
    (export "user" (type $user (eq $record)))
    (export "current" (func (result (option $user))))))
  (alias export $registry "add-route" (func $route))
  (alias export $page "set-html" (func $html))
  (alias export $users "current" (func $current))
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
  (core func $current (canon lower (func $current) (memory $memory) (realloc $realloc)))
  (core module $app
    (import "storage" "memory" (memory 2))
    (import "host" "route" (func $route (param i32 i32 i32 i32 i32 i32)))
    (import "host" "html" (func $html (param i32 i32 i32 i32)))
    ;; option<user> comes back at the pointer: the case at 0, the user at
    ;; 4 (id 4, email 12, name 20, groups 28; each pointer and length).
    (import "host" "current" (func $current (param i32)))
    ;; "GET" 0, "/" 3, "who" 4
    (data (i32.const 0) "GET/who")
    ;; "page" 32, "nobody" 36
    (data (i32.const 32) "pagenobody")
    (func (export "init")
      i32.const 0 i32.const 3 i32.const 3 i32.const 1 i32.const 4 i32.const 3 call $route)
    (func (export "start") (param $args i32)
      i32.const 512 call $current
      i32.const 512 i32.load8_u
      if
        i32.const 32 i32.const 4
        i32.const 512 i32.load offset=12 i32.const 512 i32.load offset=16
        call $html
      else
        i32.const 32 i32.const 4 i32.const 36 i32.const 6 call $html
      end))
  (core instance $app (instantiate $app
    (with "storage" (instance $storage))
    (with "host" (instance
      (export "route" (func $route)) (export "html" (func $html)) (export "current" (func $current))))))
  (func $init (canon lift (core func $app "init")))
  (func $start (param "handler" string) (param "req" $request)
    (canon lift (core func $app "start") (memory $memory) (realloc $realloc)))
  (instance $handler
    (export "request" (type $request))
    (export "init" (func $init)) (export "start" (func $start)))
  (export "appos:framework/handler@0.1.0" (instance $handler)))
