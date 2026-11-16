//! scriptc's core module -> an AppOS component.
//!
//! scriptc imports its callbacks from module `scriptc`, and a few WASI
//! preview 1 functions for its runtime. A component's core module imports
//! WIT functions under the canonical ABI instead. This rewrites the module:
//!
//!   - a callback whose core signature already is the canonical one simply
//!     becomes the WIT import (`page.set-html` takes two strings either way)
//!   - a callback with a result gets a shim: scriptc passes a result buffer
//!     as (pointer, length), the canonical ABI only the pointer
//!   - `appos_abi_mem_read` is `memory.copy`; scriptc's `panic` logs the
//!     message, then traps
//!   - WASI preview 1 becomes shims over the few preview 2 interfaces the
//!     shell allows: clocks, random; stdout and stderr go to the app log;
//!     there are no files, arguments or environment
//!   - `cabi_realloc`, which the shell calls to pass arguments, is a bump
//!     allocator on its own memory pages (each request runs in a fresh
//!     instance, so nothing is freed)
//!   - each WIT export calls scriptc's initialization once, then the
//!     scriptc export of the same name
//!
//! Callbacks the app never calls are left out, so the component imports only
//! what the app uses (an app that never calls kv needs no kv grant).

use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, anyhow, bail};
use wasm_encoder::reencode::Reencode;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType,
    ImportSection, Instruction as I, MemArg, MemorySection, Module, StartSection, TableSection, TagSection, TypeSection,
};
use wasmparser::{ExternalKind, FuncType, Operator, Parser, Payload, TypeRef, ValType};
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::abi::WasmType;
use wit_parser::{Resolve, WorldId};

use super::bindings::{MEM_READ, PREFIX};
use super::{WitFn, exports, imports};

/// The preview 2 interfaces the shims use, as much of them as they need.
const WASI_CLOCKS: &str = "package wasi:clocks@0.2.0;
interface wall-clock { record datetime { seconds: u64, nanoseconds: u32 } now: func() -> datetime; }
interface monotonic-clock { type instant = u64; now: func() -> instant; }
";
const WASI_RANDOM: &str = "package wasi:random@0.2.0;
interface random { get-random-u64: func() -> u64; }
";
pub(crate) const WALL_CLOCK: (&str, &str) = ("wasi:clocks/wall-clock@0.2.0", "now");
const MONOTONIC_CLOCK: (&str, &str) = ("wasi:clocks/monotonic-clock@0.2.0", "now");
pub(crate) const RANDOM: (&str, &str) = ("wasi:random/random@0.2.0", "get-random-u64");

/// WASI errno values.
const EBADF: i32 = 8;
const ENOSYS: i32 = 52;

pub fn componentize(core: &[u8], mut resolve: Resolve, app: WorldId) -> Result<Vec<u8>> {
    let world = with_wasi(&mut resolve, app)?;
    let mut module = rewrite(core, &imports(&resolve, world), &exports(&resolve, world))?;
    wit_component::embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false)?;
    ComponentEncoder::default().module(&module)?.validate(true).encode()
}

/// The app's world plus the WASI preview 2 functions the shims may call:
/// clocks and random, which the shell allows every app. The component
/// imports only those the module calls.
pub(crate) fn with_wasi(resolve: &mut Resolve, app: WorldId) -> Result<WorldId> {
    resolve.push_str("wasi-clocks.wit", WASI_CLOCKS)?;
    resolve.push_str("wasi-random.wit", WASI_RANDOM)?;
    let world = &resolve.worlds[app];
    let pkg = &resolve.packages[world.package.ok_or_else(|| anyhow!("the world has no package"))?].name;
    let version = pkg.version.as_ref().map(|v| format!("@{v}")).unwrap_or_default();
    let wrapper = format!(
        "package appos:core-component;\nworld component {{\n  include {}:{}/{}{version};\n  import wasi:clocks/wall-clock@0.2.0;\n  import wasi:clocks/monotonic-clock@0.2.0;\n  import wasi:random/random@0.2.0;\n}}\n",
        pkg.namespace, pkg.name, world.name
    );
    let wrapper = resolve.push_str("component.wit", &wrapper)?;
    resolve.select_world(&[wrapper], Some("component"))
}

/// What the core module has, from a first pass.
struct Core<'a> {
    types: Vec<FuncType>,
    /// Function imports: module, name, type index.
    imports: Vec<(&'a str, &'a str, u32)>,
    /// Type indices of the module's own functions.
    funcs: Vec<u32>,
    globals: u32,
    exports: Vec<(&'a str, ExternalKind, u32)>,
    /// Functions called, referenced or exported anywhere.
    used: HashSet<u32>,
}

fn parse(data: &[u8]) -> Result<Core<'_>> {
    let mut core = Core { types: vec![], imports: vec![], funcs: vec![], globals: 0, exports: vec![], used: HashSet::new() };
    for payload in Parser::new(0).parse_all(data) {
        match payload? {
            Payload::TypeSection(r) => {
                for ty in r.into_iter_err_on_gc_types() {
                    core.types.push(ty?);
                }
            }
            Payload::ImportSection(r) => {
                for import in r.into_imports() {
                    let import = import?;
                    match import.ty {
                        TypeRef::Func(t) | TypeRef::FuncExact(t) => core.imports.push((import.module, import.name, t)),
                        TypeRef::Global(_) => core.globals += 1,
                        _ => bail!("the module imports {}::{}, which isn't a function", import.module, import.name),
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    core.funcs.push(t?);
                }
            }
            Payload::GlobalSection(r) => core.globals += r.count(),
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e?;
                    if e.kind == ExternalKind::Func {
                        core.used.insert(e.index);
                    }
                    core.exports.push((e.name, e.kind, e.index));
                }
            }
            Payload::StartSection { func, .. } => {
                core.used.insert(func);
            }
            Payload::ElementSection(r) => {
                for element in r {
                    match element?.items {
                        wasmparser::ElementItems::Functions(fs) => {
                            for f in fs {
                                core.used.insert(f?);
                            }
                        }
                        wasmparser::ElementItems::Expressions(_, exprs) => {
                            for expr in exprs {
                                let mut ops = expr?.get_operators_reader();
                                while !ops.eof() {
                                    if let Operator::RefFunc { function_index } = ops.read()? {
                                        core.used.insert(function_index);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut ops = body.get_operators_reader()?;
                while !ops.eof() {
                    match ops.read()? {
                        Operator::Call { function_index } | Operator::ReturnCall { function_index } | Operator::RefFunc { function_index } => {
                            core.used.insert(function_index);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(core)
}

pub(crate) type Sig = (Vec<ValType>, Vec<ValType>);

pub(crate) fn sig_of(t: &FuncType) -> Sig {
    (t.params().to_vec(), t.results().to_vec())
}

pub(crate) fn canonical(w: &WitFn) -> Sig {
    let val = |t: &WasmType| match t {
        WasmType::I32 | WasmType::Pointer | WasmType::Length => ValType::I32,
        WasmType::I64 | WasmType::PointerOrI64 => ValType::I64,
        WasmType::F32 => ValType::F32,
        WasmType::F64 => ValType::F64,
    };
    (w.sig.params.iter().map(val).collect(), w.sig.results.iter().map(val).collect())
}

pub(crate) fn show(s: &Sig) -> String {
    format!("{:?} -> {:?}", s.0, s.1)
}

/// A function this module adds.
enum Shim {
    Trap,
    /// Calls `import` without the last parameter (the result buffer's length).
    DropLast {
        import: u32,
        params: u32,
    },
    MemRead,
    /// Logs the human text of scriptc's message, then traps.
    Panic {
        info: Option<u32>,
    },
    Errno(i32),
    ZeroSizes,
    ClockTimeGet {
        wall: u32,
        mono: u32,
    },
    RandomGet {
        random: u32,
    },
    FdWrite {
        info: Option<u32>,
        warn: Option<u32>,
    },
    Realloc,
    EnsureInit {
        initialize: Option<u32>,
        init: Option<u32>,
    },
    Export {
        target: u32,
        params: u32,
    },
}

#[derive(Default)]
struct Plan {
    /// The new function imports: module, name, signature.
    imports: Vec<(String, String, Sig)>,
    import_index: HashMap<(String, String), u32>,
    /// Shims and their type indices.
    shims: Vec<(Shim, u32)>,
    /// Types beyond the module's own.
    new_types: Vec<Sig>,
}

impl Plan {
    fn import(&mut self, module: &str, name: &str, sig: Sig) -> u32 {
        let key = (module.to_string(), name.to_string());
        if let Some(&i) = self.import_index.get(&key) {
            return i;
        }
        let i = self.imports.len() as u32;
        self.imports.push((key.0.clone(), key.1.clone(), sig));
        self.import_index.insert(key, i);
        i
    }

    fn shim(&mut self, shim: Shim, ty: u32) -> usize {
        self.shims.push((shim, ty));
        self.shims.len() - 1
    }

    fn ty(&mut self, core: &Core, sig: Sig) -> u32 {
        if let Some(i) = core.types.iter().position(|t| sig_of(t) == sig) {
            return i as u32;
        }
        if let Some(i) = self.new_types.iter().position(|t| *t == sig) {
            return (core.types.len() + i) as u32;
        }
        self.new_types.push(sig);
        (core.types.len() + self.new_types.len() - 1) as u32
    }
}

/// Where an original import goes.
enum Target {
    Import(u32),
    Shim(usize),
}

struct Remap(Vec<u32>);

impl Reencode for Remap {
    type Error = std::convert::Infallible;

    fn function_index(&mut self, func: u32) -> Result<u32, wasm_encoder::reencode::Error<Self::Error>> {
        Ok(self.0[func as usize])
    }
}

fn rewrite(data: &[u8], wit_imports: &[WitFn], wit_exports: &[WitFn]) -> Result<Vec<u8>> {
    let core = parse(data)?;
    let mut plan = Plan::default();
    let by_c_name: HashMap<&str, &WitFn> = wit_imports.iter().map(|w| (w.c_name.as_str(), w)).collect();
    let by_name: HashMap<(&str, &str), &WitFn> = wit_imports.iter().map(|w| ((w.module.as_str(), w.name.as_str()), w)).collect();
    let wit = |plan: &mut Plan, (module, name): (&str, &str)| -> Result<u32> {
        let w = by_name.get(&(module, name)).ok_or_else(|| anyhow!("the world has no {module}::{name}"))?;
        Ok(plan.import(module, name, canonical(w)))
    };
    let log = |level: &str| by_c_name.get(format!("appos_framework_log_{level}").as_str()).map(|w| (w.module.as_str(), w.name.as_str()));
    let (i32, i64) = (ValType::I32, ValType::I64);

    let mut targets = Vec::new();
    for (index, &(module, name, ty)) in core.imports.iter().enumerate() {
        let sig = sig_of(&core.types[ty as usize]);
        let expect = |want: Sig| -> Result<()> {
            if sig != want {
                bail!("{module}::{name} is {}, expected {}", show(&sig), show(&want));
            }
            Ok(())
        };
        let shim = |plan: &mut Plan, shim| Target::Shim(plan.shim(shim, ty));
        let target = if !core.used.contains(&(index as u32)) {
            shim(&mut plan, Shim::Trap)
        } else if module == "scriptc" && name == MEM_READ {
            expect((vec![i32, i32, i32], vec![]))?;
            shim(&mut plan, Shim::MemRead)
        } else if module == "scriptc" && name == "panic" {
            expect((vec![i32, i32], vec![]))?;
            let info = log("info").map(|n| wit(&mut plan, n)).transpose()?;
            shim(&mut plan, Shim::Panic { info })
        } else if module == "scriptc" {
            let w = by_c_name.get(name).ok_or_else(|| anyhow!("scriptc calls {name}, which isn't in the WIT; rebuild to regenerate bindings/"))?;
            let want = canonical(w);
            let import = plan.import(&w.module, &w.name, want.clone());
            if sig == want {
                Target::Import(import)
            } else if w.sig.retptr && sig.1.is_empty() && sig.0.len() == want.0.len() + 1 && sig.0[..want.0.len()] == want.0[..] {
                shim(&mut plan, Shim::DropLast { import, params: want.0.len() as u32 })
            } else {
                bail!("scriptc passes {name} as {}; the canonical ABI needs {}", show(&sig), show(&want));
            }
        } else if module == "wasi_snapshot_preview1" {
            match name {
                "fd_write" => {
                    expect((vec![i32, i32, i32, i32], vec![i32]))?;
                    let info = log("info").map(|n| wit(&mut plan, n)).transpose()?;
                    let warn = log("warn").map(|n| wit(&mut plan, n)).transpose()?;
                    shim(&mut plan, Shim::FdWrite { info, warn })
                }
                "clock_time_get" => {
                    expect((vec![i32, i64, i32], vec![i32]))?;
                    let (wall, mono) = (wit(&mut plan, WALL_CLOCK)?, wit(&mut plan, MONOTONIC_CLOCK)?);
                    shim(&mut plan, Shim::ClockTimeGet { wall, mono })
                }
                "random_get" => {
                    expect((vec![i32, i32], vec![i32]))?;
                    let random = wit(&mut plan, RANDOM)?;
                    shim(&mut plan, Shim::RandomGet { random })
                }
                "args_sizes_get" | "environ_sizes_get" => {
                    expect((vec![i32, i32], vec![i32]))?;
                    shim(&mut plan, Shim::ZeroSizes)
                }
                "args_get" | "environ_get" | "sched_yield" if sig.1 == [i32] => shim(&mut plan, Shim::Errno(0)),
                _ if sig.1 == [i32] => {
                    let errno = if name.starts_with("fd_") || name.starts_with("path_") { EBADF } else { ENOSYS };
                    shim(&mut plan, Shim::Errno(errno))
                }
                _ => shim(&mut plan, Shim::Trap),
            }
        } else {
            bail!("the module imports {module}::{name}, which AppOS doesn't provide");
        };
        targets.push(target);
    }

    // Exports: memory, cabi_realloc, and the WIT exports behind one init.
    let export = |name: &str, kind: ExternalKind| core.exports.iter().find(|e| e.0 == name && e.1 == kind).map(|e| e.2);
    let memory = export("memory", ExternalKind::Memory).ok_or_else(|| anyhow!("the module exports no memory"))?;
    let init_ty = plan.ty(&core, (vec![], vec![]));
    let ensure_init = plan
        .shim(Shim::EnsureInit { initialize: export("_initialize", ExternalKind::Func), init: export(&format!("{PREFIX}_init"), ExternalKind::Func) }, init_ty);
    let realloc_ty = plan.ty(&core, (vec![i32; 4], vec![i32]));
    let realloc = plan.shim(Shim::Realloc, realloc_ty);
    let mut wrappers = Vec::new();
    for w in wit_exports {
        let target = export(&w.c_name, ExternalKind::Func).ok_or_else(|| anyhow!("scriptc's module has no export {}", w.c_name))?;
        let local = (target as usize).checked_sub(core.imports.len()).ok_or_else(|| anyhow!("{} is an import", w.c_name))?;
        let ty = core.funcs[local];
        let (have, want) = (sig_of(&core.types[ty as usize]), canonical(w));
        if have != want {
            bail!("scriptc exports {} as {}; the canonical ABI needs {}", w.c_name, show(&have), show(&want));
        }
        let shim = plan.shim(Shim::Export { target, params: want.0.len() as u32 }, ty);
        wrappers.push((w.name.clone(), shim));
    }

    // Function indices: new imports, then the module's functions, then shims.
    let new_imports = plan.imports.len() as u32;
    let shim_base = new_imports + core.funcs.len() as u32;
    let shim_index = |s: usize| shim_base + s as u32;
    let mut map: Vec<u32> = targets
        .iter()
        .map(|t| match t {
            Target::Import(i) => *i,
            Target::Shim(s) => shim_index(*s),
        })
        .collect();
    map.extend((0..core.funcs.len() as u32).map(|j| new_imports + j));
    let mut remap = Remap(map);

    let globals = core.globals;
    let (init_done, heap_next, heap_end, scratch) = (globals, globals + 1, globals + 2, globals + 3);
    let realloc_fn = shim_index(realloc);
    let ensure_fn = shim_index(ensure_init);
    let bodies: Vec<Function> = plan
        .shims
        .iter()
        .map(|(shim, _)| {
            let local = |remap: &Remap, f: u32| remap.0[f as usize];
            match shim {
                Shim::Trap => body(&[], &[I::Unreachable]),
                Shim::DropLast { import, params } => {
                    let mut ins: Vec<I> = (0..*params).map(I::LocalGet).collect();
                    ins.push(I::Call(*import));
                    body(&[], &ins)
                }
                Shim::MemRead => body(&[], &[I::LocalGet(1), I::LocalGet(0), I::LocalGet(2), I::MemoryCopy { src_mem: 0, dst_mem: 0 }]),
                // scriptc's runtime errors are structured, 0x01 text 0x1F
                // code 0x1F symbol [0x1F remediation]; the log gets the text,
                // a plain line like a Rust panic's, without a final newline.
                // (ptr, len), then local 2 counts.
                Shim::Panic { info } => {
                    let mut ins = Vec::new();
                    if let Some(info) = info {
                        ins.extend([
                            I::LocalGet(1),
                            I::If(BlockType::Empty),
                            I::LocalGet(0),
                            I::I32Load8U(mem(0, 0)),
                            I::I32Const(1),
                            I::I32Eq,
                            I::If(BlockType::Empty),
                            I::LocalGet(0),
                            I::I32Const(1),
                            I::I32Add,
                            I::LocalSet(0),
                            I::LocalGet(1),
                            I::I32Const(1),
                            I::I32Sub,
                            I::LocalSet(1),
                            I::Block(BlockType::Empty),
                            I::Loop(BlockType::Empty),
                            I::LocalGet(2),
                            I::LocalGet(1),
                            I::I32GeU,
                            I::BrIf(1),
                            I::LocalGet(0),
                            I::LocalGet(2),
                            I::I32Add,
                            I::I32Load8U(mem(0, 0)),
                            I::I32Const(0x1f),
                            I::I32Eq,
                            I::BrIf(1),
                            I::LocalGet(2),
                            I::I32Const(1),
                            I::I32Add,
                            I::LocalSet(2),
                            I::Br(0),
                            I::End,
                            I::End,
                            I::LocalGet(2),
                            I::LocalSet(1),
                            I::End,
                            I::End,
                            I::Block(BlockType::Empty),
                            I::Loop(BlockType::Empty),
                            I::LocalGet(1),
                            I::I32Eqz,
                            I::BrIf(1),
                            I::LocalGet(0),
                            I::LocalGet(1),
                            I::I32Add,
                            I::I32Const(1),
                            I::I32Sub,
                            I::I32Load8U(mem(0, 0)),
                            I::I32Const(10),
                            I::I32Ne,
                            I::BrIf(1),
                            I::LocalGet(1),
                            I::I32Const(1),
                            I::I32Sub,
                            I::LocalSet(1),
                            I::Br(0),
                            I::End,
                            I::End,
                            I::LocalGet(0),
                            I::LocalGet(1),
                            I::Call(*info),
                        ]);
                    }
                    ins.push(I::Unreachable);
                    body(&[ValType::I32], &ins)
                }
                Shim::Errno(e) => body(&[], &[I::I32Const(*e)]),
                Shim::ZeroSizes => {
                    body(&[], &[I::LocalGet(0), I::I32Const(0), I::I32Store(mem(0, 2)), I::LocalGet(1), I::I32Const(0), I::I32Store(mem(0, 2)), I::I32Const(0)])
                }
                Shim::ClockTimeGet { wall, mono } => body(
                    &[],
                    &[
                        I::LocalGet(0),
                        I::If(BlockType::Empty),
                        I::LocalGet(2),
                        I::Call(*mono),
                        I::I64Store(mem(0, 3)),
                        I::Else,
                        I::GlobalGet(scratch),
                        I::I32Eqz,
                        I::If(BlockType::Empty),
                        I::I32Const(0),
                        I::I32Const(0),
                        I::I32Const(8),
                        I::I32Const(16),
                        I::Call(realloc_fn),
                        I::GlobalSet(scratch),
                        I::End,
                        I::GlobalGet(scratch),
                        I::Call(*wall),
                        I::LocalGet(2),
                        I::GlobalGet(scratch),
                        I::I64Load(mem(0, 3)),
                        I::I64Const(1_000_000_000),
                        I::I64Mul,
                        I::GlobalGet(scratch),
                        I::I64Load32U(mem(8, 2)),
                        I::I64Add,
                        I::I64Store(mem(0, 3)),
                        I::End,
                        I::I32Const(0),
                    ],
                ),
                Shim::RandomGet { random } => body(
                    &[ValType::I32, ValType::I64],
                    &[
                        I::Block(BlockType::Empty),
                        I::Loop(BlockType::Empty),
                        I::LocalGet(2),
                        I::LocalGet(1),
                        I::I32GeU,
                        I::BrIf(1),
                        I::LocalGet(2),
                        I::I32Const(7),
                        I::I32And,
                        I::I32Eqz,
                        I::If(BlockType::Empty),
                        I::Call(*random),
                        I::LocalSet(3),
                        I::End,
                        I::LocalGet(0),
                        I::LocalGet(2),
                        I::I32Add,
                        I::LocalGet(3),
                        I::I64Store8(mem(0, 0)),
                        I::LocalGet(3),
                        I::I64Const(8),
                        I::I64ShrU,
                        I::LocalSet(3),
                        I::LocalGet(2),
                        I::I32Const(1),
                        I::I32Add,
                        I::LocalSet(2),
                        I::Br(0),
                        I::End,
                        I::End,
                        I::I32Const(0),
                    ],
                ),
                Shim::FdWrite { info, warn } => {
                    // Locals: 4 i, 5 total, 6 iovec, 7 ptr, 8 len.
                    let mut ins = vec![
                        // Only stdout (1) and stderr (2).
                        I::LocalGet(0),
                        I::I32Const(1),
                        I::I32Sub,
                        I::I32Const(1),
                        I::I32GtU,
                        I::If(BlockType::Empty),
                        I::I32Const(EBADF),
                        I::Return,
                        I::End,
                        I::Block(BlockType::Empty),
                        I::Loop(BlockType::Empty),
                        I::LocalGet(4),
                        I::LocalGet(2),
                        I::I32GeU,
                        I::BrIf(1),
                        I::LocalGet(1),
                        I::LocalGet(4),
                        I::I32Const(3),
                        I::I32Shl,
                        I::I32Add,
                        I::LocalSet(6),
                        I::LocalGet(6),
                        I::I32Load(mem(0, 2)),
                        I::LocalSet(7),
                        I::LocalGet(6),
                        I::I32Load(mem(4, 2)),
                        I::LocalSet(8),
                        I::LocalGet(5),
                        I::LocalGet(8),
                        I::I32Add,
                        I::LocalSet(5),
                        // A log line has no trailing newline.
                        I::LocalGet(8),
                        I::If(BlockType::Empty),
                        I::LocalGet(7),
                        I::LocalGet(8),
                        I::I32Add,
                        I::I32Const(1),
                        I::I32Sub,
                        I::I32Load8U(mem(0, 0)),
                        I::I32Const(10),
                        I::I32Eq,
                        I::If(BlockType::Empty),
                        I::LocalGet(8),
                        I::I32Const(1),
                        I::I32Sub,
                        I::LocalSet(8),
                        I::End,
                        I::End,
                        I::LocalGet(8),
                        I::If(BlockType::Empty),
                    ];
                    match (info, warn) {
                        (Some(info), Some(warn)) => ins.extend([
                            I::LocalGet(0),
                            I::I32Const(2),
                            I::I32Eq,
                            I::If(BlockType::Empty),
                            I::LocalGet(7),
                            I::LocalGet(8),
                            I::Call(*warn),
                            I::Else,
                            I::LocalGet(7),
                            I::LocalGet(8),
                            I::Call(*info),
                            I::End,
                        ]),
                        (Some(log), None) | (None, Some(log)) => ins.extend([I::LocalGet(7), I::LocalGet(8), I::Call(*log)]),
                        (None, None) => {}
                    }
                    ins.extend([
                        I::End,
                        I::LocalGet(4),
                        I::I32Const(1),
                        I::I32Add,
                        I::LocalSet(4),
                        I::Br(0),
                        I::End,
                        I::End,
                        I::LocalGet(3),
                        I::LocalGet(5),
                        I::I32Store(mem(0, 2)),
                        I::I32Const(0),
                    ]);
                    body(&[ValType::I32; 5], &ins)
                }
                Shim::Realloc => body(
                    // Params: 0 old ptr, 1 old size, 2 align, 3 new size. Locals: 4 ptr, 5 pages.
                    &[ValType::I32, ValType::I32],
                    &[
                        I::GlobalGet(heap_next),
                        I::LocalGet(2),
                        I::I32Add,
                        I::I32Const(1),
                        I::I32Sub,
                        I::I32Const(0),
                        I::LocalGet(2),
                        I::I32Sub,
                        I::I32And,
                        I::LocalSet(4),
                        I::LocalGet(4),
                        I::LocalGet(3),
                        I::I32Add,
                        I::GlobalGet(heap_end),
                        I::I32GtU,
                        I::If(BlockType::Empty),
                        I::LocalGet(3),
                        I::LocalGet(2),
                        I::I32Add,
                        I::I32Const(65535),
                        I::I32Add,
                        I::I32Const(16),
                        I::I32ShrU,
                        I::LocalTee(5),
                        I::MemoryGrow(0),
                        I::LocalTee(4),
                        I::I32Const(-1),
                        I::I32Eq,
                        I::If(BlockType::Empty),
                        I::Unreachable,
                        I::End,
                        I::LocalGet(4),
                        I::LocalGet(5),
                        I::I32Add,
                        I::I32Const(16),
                        I::I32Shl,
                        I::GlobalSet(heap_end),
                        I::LocalGet(4),
                        I::I32Const(16),
                        I::I32Shl,
                        I::LocalGet(2),
                        I::I32Add,
                        I::I32Const(1),
                        I::I32Sub,
                        I::I32Const(0),
                        I::LocalGet(2),
                        I::I32Sub,
                        I::I32And,
                        I::LocalSet(4),
                        I::End,
                        I::LocalGet(4),
                        I::LocalGet(3),
                        I::I32Add,
                        I::GlobalSet(heap_next),
                        I::LocalGet(0),
                        I::If(BlockType::Empty),
                        I::LocalGet(4),
                        I::LocalGet(0),
                        I::LocalGet(1),
                        I::LocalGet(3),
                        I::LocalGet(1),
                        I::LocalGet(3),
                        I::I32LtU,
                        I::Select,
                        I::MemoryCopy { src_mem: 0, dst_mem: 0 },
                        I::End,
                        I::LocalGet(4),
                    ],
                ),
                Shim::EnsureInit { initialize, init } => {
                    let mut ins = vec![I::GlobalGet(init_done), I::I32Eqz, I::If(BlockType::Empty), I::I32Const(1), I::GlobalSet(init_done)];
                    ins.extend(initialize.iter().chain(init).map(|f| I::Call(local(&remap, *f))));
                    ins.push(I::End);
                    body(&[], &ins)
                }
                Shim::Export { target, params } => {
                    let mut ins = vec![I::Call(ensure_fn)];
                    ins.extend((0..*params).map(I::LocalGet));
                    ins.push(I::Call(local(&remap, *target)));
                    body(&[], &ins)
                }
            }
        })
        .collect();

    // Second pass: copy the module with the new indices, add the rest.
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut tables = None;
    let mut memories = None;
    let mut tags = None;
    let mut global_section = GlobalSection::new();
    let mut start = None;
    let mut elements = None;
    let mut data_count = None;
    let mut code = CodeSection::new();
    let mut data_section = None;
    let mut other_imports = Vec::new();
    for payload in Parser::new(0).parse_all(data) {
        match payload? {
            Payload::TypeSection(r) => remap.parse_type_section(&mut types, r)?,
            Payload::ImportSection(r) => {
                for import in r.into_imports() {
                    let import = import?;
                    if let TypeRef::Global(g) = import.ty {
                        other_imports.push((import.module.to_string(), import.name.to_string(), remap.global_type(g)?));
                    }
                }
            }
            Payload::FunctionSection(r) => remap.parse_function_section(&mut functions, r)?,
            Payload::TableSection(r) => remap.parse_table_section(tables.get_or_insert_with(TableSection::new), r)?,
            Payload::MemorySection(r) => remap.parse_memory_section(memories.get_or_insert_with(MemorySection::new), r)?,
            Payload::TagSection(r) => remap.parse_tag_section(tags.get_or_insert_with(TagSection::new), r)?,
            Payload::GlobalSection(r) => remap.parse_global_section(&mut global_section, r)?,
            Payload::StartSection { func, .. } => start = Some(remap.function_index(func)?),
            Payload::ElementSection(r) => remap.parse_element_section(elements.get_or_insert_with(ElementSection::new), r)?,
            Payload::DataCountSection { count, .. } => data_count = Some(count),
            Payload::DataSection(r) => remap.parse_data_section(data_section.get_or_insert_with(DataSection::new), r)?,
            Payload::CodeSectionStart { range, .. } => {
                let (from, to) = (range.start as usize, range.end as usize);
                let reader = wasmparser::BinaryReader::new(&data[from..to], range.start);
                remap.parse_code_section(&mut code, wasmparser::CodeSectionReader::new(reader)?)?;
            }
            _ => {}
        }
    }
    let import_sigs: Vec<Sig> = plan.imports.iter().map(|(_, _, sig)| sig.clone()).collect();
    let import_types: Vec<u32> = import_sigs.into_iter().map(|sig| plan.ty(&core, sig)).collect();
    for (params, results) in &plan.new_types {
        types.ty().function(params.iter().map(|t| encode(*t)), results.iter().map(|t| encode(*t)));
    }
    let mut import_section = ImportSection::new();
    for ((module, name, _), ty) in plan.imports.iter().zip(import_types) {
        import_section.import(module, name, wasm_encoder::EntityType::Function(ty));
    }
    for (module, name, ty) in other_imports {
        import_section.import(&module, &name, ty);
    }
    for (_, ty) in &plan.shims {
        functions.function(*ty);
    }
    for f in &bodies {
        code.function(f);
    }
    for _ in [init_done, heap_next, heap_end, scratch] {
        global_section.global(GlobalType { val_type: wasm_encoder::ValType::I32, mutable: true, shared: false }, &ConstExpr::i32_const(0));
    }
    let mut export_section = ExportSection::new();
    export_section.export("memory", ExportKind::Memory, memory);
    export_section.export("cabi_realloc", ExportKind::Func, realloc_fn);
    for (name, shim) in &wrappers {
        export_section.export(name, ExportKind::Func, shim_index(*shim));
    }

    let mut module = Module::new();
    module.section(&types);
    module.section(&import_section);
    module.section(&functions);
    if let Some(s) = &tables {
        module.section(s);
    }
    if let Some(s) = &memories {
        module.section(s);
    }
    if let Some(s) = &tags {
        module.section(s);
    }
    module.section(&global_section);
    module.section(&export_section);
    if let Some(function_index) = start {
        module.section(&StartSection { function_index });
    }
    if let Some(s) = &elements {
        module.section(s);
    }
    if let Some(count) = data_count {
        module.section(&wasm_encoder::DataCountSection { count });
    }
    module.section(&code);
    if let Some(s) = &data_section {
        module.section(s);
    }
    let bytes = module.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all()).validate_all(&bytes).context("the adapted module doesn't validate")?;
    Ok(bytes)
}

fn encode(t: ValType) -> wasm_encoder::ValType {
    match t {
        ValType::I32 => wasm_encoder::ValType::I32,
        ValType::I64 => wasm_encoder::ValType::I64,
        ValType::F32 => wasm_encoder::ValType::F32,
        ValType::F64 => wasm_encoder::ValType::F64,
        other => unreachable!("shims use only number types, not {other:?}"),
    }
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg { offset, align, memory_index: 0 }
}

fn body(locals: &[ValType], ins: &[I]) -> Function {
    let mut f = Function::new(locals.iter().map(|t| (1, encode(*t))));
    for i in ins {
        f.instruction(i);
    }
    f.instruction(&I::End);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module shaped like scriptc's: callbacks from `scriptc`, a few WASI
    /// preview 1 functions, exports by C name.
    const CORE: &str = r#"(module
        (import "scriptc" "appos_abi_mem_read" (func $mem_read (param i32 i32 i32)))
        (import "scriptc" "appos_framework_page_set_html" (func $set_html (param i32 i32 i32 i32)))
        (import "scriptc" "appos_framework_kv_get" (func $kv_get (param i32 i32 i32 i32)))
        (import "scriptc" "panic" (func $panic (param i32 i32)))
        (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "clock_time_get" (func $clock (param i32 i64 i32) (result i32)))
        (import "wasi_snapshot_preview1" "environ_get" (func $environ_get (param i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
        (memory (export "memory") 1)
        (func (export "_initialize"))
        (func (export "app_init"))
        (func (export "scriptc_alloc") (param i32) (result i32) (i32.const 0))
        (func (export "appos_framework_handler_init"))
        (func (export "appos_framework_handler_start") (param i32)
          (call $mem_read (i32.const 0) (i32.const 0) (i32.const 0))
          (call $set_html (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))
          (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 0) (i32.const 0)))
          (drop (call $clock (i32.const 0) (i64.const 0) (i32.const 0)))
          (drop (call $environ_get (i32.const 0) (i32.const 0)))
          (call $panic (i32.const 0) (i32.const 0))))"#;

    fn appos() -> (Resolve, WorldId) {
        let mut resolve = Resolve::new();
        let pkg = resolve.push_str("appos.wit", include_str!("../../../wit/appos.wit")).unwrap();
        let world = resolve.select_world(&[pkg], None).unwrap();
        (resolve, world)
    }

    fn component_imports(component: &[u8]) -> Vec<String> {
        // Only the outer component's imports: nested ones are wit-component's.
        let (mut names, mut depth) = (Vec::new(), 0);
        for payload in Parser::new(0).parse_all(component) {
            match payload.unwrap() {
                Payload::Version { .. } => depth += 1,
                Payload::End(_) => depth -= 1,
                Payload::ComponentImportSection(r) if depth == 1 => {
                    for import in r {
                        names.push(import.unwrap().name.name.to_string());
                    }
                }
                _ => {}
            }
        }
        names.sort();
        names
    }

    #[test]
    fn a_scriptc_module_becomes_a_component_that_imports_only_what_it_calls() {
        let (resolve, world) = appos();
        let component = componentize(&wat::parse_str(CORE).unwrap(), resolve, world).unwrap();
        // kv.get is declared but never called: no kv import, no grant needed.
        // No wasi:cli or filesystem either: the shell allows none of that.
        assert_eq!(
            component_imports(&component),
            [
                "appos:framework/log@0.1.0",
                "appos:framework/page@0.1.0",
                "appos:framework/types@0.1.0",
                "wasi:clocks/monotonic-clock@0.2.0",
                "wasi:clocks/wall-clock@0.2.0",
            ]
        );
    }

    #[test]
    fn a_callback_in_the_wrong_shape_is_an_error() {
        let (resolve, world) = appos();
        let core = CORE.replace(r#"(func $set_html (param i32 i32 i32 i32))"#, r#"(func $set_html (param i32 i32))"#);
        let err = format!("{:#}", componentize(&wat::parse_str(&core).unwrap(), resolve, world).unwrap_err());
        assert!(err.contains("scriptc passes appos_framework_page_set_html as [I32, I32]"), "{err}");
    }

    #[test]
    fn an_import_from_elsewhere_is_refused() {
        let (resolve, world) = appos();
        let core = CORE.replace(r#"(import "scriptc" "panic""#, r#"(import "env" "socket""#);
        let err = format!("{:#}", componentize(&wat::parse_str(&core).unwrap(), resolve, world).unwrap_err());
        assert!(err.contains("imports env::socket, which AppOS doesn't provide"), "{err}");
    }
}
