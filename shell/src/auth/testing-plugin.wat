;; A sign-in plugin for the tests (plugin.rs). `start` shows "plugin page".
;; `finish` fetches config "url" and signs in as the email the answer's
;; body is, or refuses with fetch's error. Without a "url" it traps.
(component
  (type $pairs (list (tuple string string)))
  (import "appos:auth/config@0.1.0" (instance $config
    (export "get" (func (param "key" string) (result (option string))))))
  (import "appos:auth/http@0.1.0" (instance $http
    (type $pairs (list (tuple string string)))
    (type $req (record (field "method" string) (field "url" string) (field "headers" $pairs) (field "body" string)))
    (export "request" (type $request (eq $req)))
    (type $resp (record (field "status" u16) (field "headers" $pairs) (field "body" string)))
    (export "response" (type $response (eq $resp)))
    (export "fetch" (func (param "req" $request) (result (result $response (error string)))))))
  (alias export $config "get" (func $get))
  (alias export $http "fetch" (func $fetch))
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
  (core func $get (canon lower (func $get) (memory $memory) (realloc $realloc)))
  (core func $fetch (canon lower (func $fetch) (memory $memory) (realloc $realloc)))
  (core module $plugin
    (import "storage" "memory" (memory 2))
    (import "host" "get" (func $get (param i32 i32 i32)))
    (import "host" "fetch" (func $fetch (param i32 i32 i32 i32 i32 i32 i32 i32 i32)))
    ;; "url" 0, "GET" 3, "plugin page" 16
    (data (i32.const 0) "urlGET")
    (data (i32.const 16) "plugin page")
    ;; A step: the case at +0, then its string (pointer, length) or the
    ;; identity (id, email, name, groups as pointer, length each) from +4.
    ;; start's step 128, config.get's answer 160, fetch's 176, finish's 208.
    (func (export "start") (param i32) (result i32)
      i32.const 128 i32.const 0 i32.store8
      i32.const 132 i32.const 16 i32.store
      i32.const 136 i32.const 11 i32.store
      i32.const 128)
    (func (export "finish") (param i32) (result i32)
      i32.const 0 i32.const 3 i32.const 160 call $get
      i32.const 160 i32.load8_u i32.eqz
      if unreachable end
      ;; fetch({method: "GET", url, headers: [], body: ""})
      i32.const 3 i32.const 3
      i32.const 160 i32.load offset=4 i32.const 160 i32.load offset=8
      i32.const 0 i32.const 0
      i32.const 0 i32.const 0
      i32.const 176
      call $fetch
      ;; err(message): its bytes after the case are refuse(message)'s.
      i32.const 176 i32.load8_u
      if
        i32.const 176 i32.const 3 i32.store8
        i32.const 176 return
      end
      ;; ok(response): done({id: "", email: body, name: "", groups: []});
      ;; the body is at +16 of the answer.
      i32.const 208 i32.const 2 i32.store8
      i32.const 212 i32.const 0 i32.store
      i32.const 216 i32.const 0 i32.store
      i32.const 220 i32.const 176 i32.load offset=16 i32.store
      i32.const 224 i32.const 176 i32.load offset=20 i32.store
      i32.const 228 i32.const 0 i32.store
      i32.const 232 i32.const 0 i32.store
      i32.const 236 i32.const 0 i32.store
      i32.const 240 i32.const 0 i32.store
      i32.const 208))
  (core instance $plugin (instantiate $plugin
    (with "storage" (instance $storage))
    (with "host" (instance (export "get" (func $get)) (export "fetch" (func $fetch))))))
  (type $identity (record
    (field "id" string) (field "email" string) (field "name" string) (field "groups" (list string))))
  (type $request (record
    (field "method" string) (field "path" string) (field "query" $pairs) (field "form" $pairs)
    (field "origin" string) (field "callback" string) (field "state" string) (field "nonce" string)
    (field "verifier" string) (field "challenge" string)))
  (type $step (variant
    (case "page" string) (case "redirect" string) (case "done" $identity)
    (case "refuse" string) (case "unavailable" string)))
  (func $start (param "req" $request) (result $step)
    (canon lift (core func $plugin "start") (memory $memory) (realloc $realloc)))
  (func $finish (param "req" $request) (result $step)
    (canon lift (core func $plugin "finish") (memory $memory) (realloc $realloc)))
  (instance $provider
    (export "identity" (type $identity))
    (export "request" (type $request))
    (export "step" (type $step))
    (export "start" (func $start)) (export "finish" (func $finish)))
  (export "appos:auth/provider@0.1.0" (instance $provider)))
