;; A kit in Wasm that fetches, for the fetch tests (wasm_kit.rs): it offers
;; acme:crm's leads. count-leads GETs {url} and answers 7 when that answers
;; 200, else 0; the command add-lead(source) POSTs source to {url}. The
;; tests put a service's URL in place of {url}, its length for {len}.
(component
  (import "appos:framework/http@0.1.0" (instance $http
    (type $pairs (list (tuple string string)))
    (type $req (record (field "method" string) (field "url" string) (field "headers" $pairs) (field "body" string)))
    (export "request" (type $request (eq $req)))
    (type $resp (record (field "status" u16) (field "headers" $pairs) (field "body" string)))
    (export "response" (type $response (eq $resp)))
    (export "fetch" (func (param "req" $request) (result (result $response (error string)))))))
  (alias export $http "fetch" (func $fetch))
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
  (core func $fetch (canon lower (func $fetch) (memory $memory) (realloc $realloc)))
  (core module $kit
    (import "storage" "memory" (memory 1))
    (import "host" "fetch" (func $fetch (param i32 i32 i32 i32 i32 i32 i32 i32 i32)))
    ;; "GET" 0, "POST" 3, the URL 16. fetch's answer goes to 512: the case
    ;; at +0, then the status at +4.
    (data (i32.const 0) "GETPOST")
    (data (i32.const 16) "{url}")
    (func (export "count") (param i32 i32) (result i32)
      ;; fetch({method, url, headers: [], body: ""})
      i32.const 0 i32.const 3 ;; GET
      i32.const 16 i32.const {len}
      i32.const 0 i32.const 0
      i32.const 0 i32.const 0
      i32.const 512
      call $fetch
      i32.const 512 i32.load8_u
      if (result i32)
        i32.const 0
      else
        i32.const 512 i32.load16_u offset=4
        i32.const 200 i32.eq
        if (result i32) i32.const 7 else i32.const 0 end
      end)
    (func (export "add") (param $ptr i32) (param $len i32)
      i32.const 3 i32.const 4
      i32.const 16 i32.const {len}
      i32.const 0 i32.const 0
      local.get $ptr local.get $len
      i32.const 512
      call $fetch))
  (core instance $kit (instantiate $kit
    (with "storage" (instance $storage))
    (with "host" (instance (export "fetch" (func $fetch))))))
  (func $count (param "source" string) (result u32)
    (canon lift (core func $kit "count") (memory $memory) (realloc $realloc)))
  (func $add (param "source" string)
    (canon lift (core func $kit "add") (memory $memory) (realloc $realloc)))
  (instance $leads (export "count-leads" (func $count)) (export "add-lead" (func $add)))
  (export "acme:crm/leads@1.0.0" (instance $leads)))
