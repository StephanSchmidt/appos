//! TeaVM's module -> a core module for wit-component.
//!
//! TeaVM's WebAssembly GC output expects its JavaScript runtime: it imports
//! JS string builtins, JS interop, the clock, Math, the console and its
//! linear memory. This replaces each with a function or global of the
//! module itself:
//!
//!   - wasm:js-string: TeaVM turns class names and messages into JS
//!     strings. Here such a string is an array of UTF-16 code units (the
//!     type of a Java char[]) behind externref.
//!   - teavmJso, teavm: JS interop and stack traces, which an app on AppOS
//!     can't reach: empty values.
//!   - teavmDate.currentTimeMillis: wasi:clocks/wall-clock.
//!   - teavmMath: random from wasi:random; floor, ceil, sqrt and the like
//!     as instructions. Others (sin, pow) are a build error naming them.
//!   - teavmConsole: System.out and System.err, line by line, into the app
//!     log (info and warn).
//!   - teavmMemory: TeaVM's heap starts after its data.
//!   - env.memory: defined here and exported, as the canonical ABI needs.
//!
//! The world's imports stay imports. Exports: the world's functions and
//! cabi_realloc (both from the generated Wit class) and the memory; TeaVM's
//! own (teavm.*, main) are dropped, and so are its custom sections.

use std::collections::HashMap;

use anyhow::{Context as _, Result, anyhow, bail};
use wasm_encoder::reencode::Reencode;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType, HeapType,
    ImportSection, Instruction as I, MemArg, MemorySection, Module, RefType, StartSection, TableSection, TagSection, TypeSection,
};
use wasmparser::{CompositeInnerType, DataKind, ExternalKind, Operator, Parser, Payload, StorageType, TypeRef, ValType};

use crate::ts::WitFn;
use crate::ts::adapt::{RANDOM, Sig, WALL_CLOCK, canonical, show, sig_of};

/// TeaVM's custom section that says how much data it keeps at address 0.
const MEMORY_REQUIREMENTS: &str = "teavm.memoryRequirements";
/// The most characters a line of System.out holds before it goes to the
/// log anyway.
const LINE: i32 = 1024;

/// What TeaVM's module has, from a first pass.
struct Core<'a> {
    /// Signatures by type index; None for GC struct and array types.
    types: Vec<Option<Sig>>,
    /// The first (array (mut i16)): a Java char[], and here a JS string.
    chars: Option<u32>,
    /// Function imports: module, name, type index.
    imports: Vec<(&'a str, &'a str, u32)>,
    /// Imported globals: module, name, type.
    globals: Vec<(&'a str, &'a str, wasmparser::GlobalType)>,
    memory: Option<wasmparser::MemoryType>,
    /// Type indices of the module's own functions.
    funcs: Vec<u32>,
    defined_globals: u32,
    exports: Vec<(&'a str, ExternalKind, u32)>,
    /// Where TeaVM's data ends in linear memory.
    data_end: u64,
}

fn parse(data: &[u8]) -> Result<Core<'_>> {
    let mut core =
        Core { types: vec![], chars: None, imports: vec![], globals: vec![], memory: None, funcs: vec![], defined_globals: 0, exports: vec![], data_end: 0 };
    for payload in Parser::new(0).parse_all(data) {
        match payload? {
            Payload::TypeSection(r) => {
                for group in r {
                    for sub in group?.into_types() {
                        let index = core.types.len() as u32;
                        match &sub.composite_type.inner {
                            CompositeInnerType::Func(f) => core.types.push(Some(sig_of(f))),
                            CompositeInnerType::Array(a) => {
                                if core.chars.is_none() && a.0.mutable && a.0.element_type == StorageType::I16 {
                                    core.chars = Some(index);
                                }
                                core.types.push(None);
                            }
                            _ => core.types.push(None),
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for import in r.into_imports() {
                    let import = import?;
                    match import.ty {
                        TypeRef::Func(t) | TypeRef::FuncExact(t) => core.imports.push((import.module, import.name, t)),
                        TypeRef::Global(g) => core.globals.push((import.module, import.name, g)),
                        TypeRef::Memory(m) if core.memory.is_none() => core.memory = Some(m),
                        _ => bail!("TeaVM's module imports {}#{}, which AppOS doesn't provide", import.module, import.name),
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    core.funcs.push(t?);
                }
            }
            Payload::MemorySection(r) if r.count() > 0 => bail!("TeaVM's module defines its own memory; expected it imported from env"),
            Payload::GlobalSection(r) => core.defined_globals = r.count(),
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e?;
                    core.exports.push((e.name, e.kind, e.index));
                }
            }
            Payload::DataSection(r) => {
                for segment in r {
                    let segment = segment?;
                    if let DataKind::Active { offset_expr, .. } = segment.kind
                        && let Operator::I32Const { value } = offset_expr.get_operators_reader().read()?
                    {
                        core.data_end = core.data_end.max(value as u32 as u64 + segment.data.len() as u64);
                    }
                }
            }
            Payload::CustomSection(r) if r.name() == MEMORY_REQUIREMENTS => {
                let json: serde_json::Value = serde_json::from_slice(r.data()).context("TeaVM's memory requirements aren't JSON")?;
                let size = json.get("dataSize").and_then(|n| n.as_u64()).unwrap_or(0);
                core.data_end = core.data_end.max(size);
            }
            _ => {}
        }
    }
    Ok(core)
}

impl Core<'_> {
    fn sig(&self, ty: u32) -> Result<&Sig> {
        self.types.get(ty as usize).and_then(|t| t.as_ref()).ok_or_else(|| anyhow!("type {ty} isn't a function type"))
    }

    fn export(&self, name: &str, kind: ExternalKind) -> Option<u32> {
        self.exports.iter().find(|e| e.0 == name && e.1 == kind).map(|e| e.2)
    }
}

/// A function this module adds in place of a runtime import.
enum Shim {
    /// An empty value of the result type: JS interop, stack traces.
    Empty,
    JsString(JsString),
    /// teavmDate.currentTimeMillis from the wall clock.
    Clock {
        now: u32,
        scratch: u32,
    },
    /// Math.random from wasi:random.
    Random {
        random: u32,
    },
    /// A Math function as instructions over its parameters.
    Math(Vec<I<'static>>),
    /// One character of System.out or System.err: lines go to the log.
    Putchar {
        log: Option<u32>,
        buf: u32,
        len: u32,
    },
}

/// The JS string builtins TeaVM uses, over arrays of UTF-16 code units.
enum JsString {
    FromCharCode,
    Concat,
    Substring,
    /// From a Java char[] of this type.
    FromCharCodeArray(u32),
    /// Into a Java char[] of this type.
    IntoCharCodeArray(u32),
    Length,
    CharCodeAt,
}

#[derive(Default)]
struct Plan {
    /// Function imports of the new module: module, name, type index.
    imports: Vec<(String, String, u32)>,
    shims: Vec<(Shim, u32)>,
    /// Function types beyond TeaVM's.
    new_types: Vec<Sig>,
    /// Mutable i32 globals beyond TeaVM's, starting at 0.
    new_globals: u32,
}

impl Plan {
    fn import(&mut self, module: &str, name: &str, ty: u32) -> u32 {
        if let Some(i) = self.imports.iter().position(|(m, n, _)| m == module && n == name) {
            return i as u32;
        }
        self.imports.push((module.to_string(), name.to_string(), ty));
        self.imports.len() as u32 - 1
    }

    fn ty(&mut self, core: &Core, sig: Sig) -> u32 {
        let index = match self.new_types.iter().position(|t| *t == sig) {
            Some(i) => i,
            None => {
                self.new_types.push(sig);
                self.new_types.len() - 1
            }
        };
        (core.types.len() + index) as u32
    }

    fn global(&mut self, core: &Core) -> u32 {
        self.new_globals += 1;
        core.globals.len() as u32 + core.defined_globals + self.new_globals - 1
    }
}

/// Where an original function import goes.
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

pub fn rewrite(data: &[u8], wit_imports: &[WitFn], wit_exports: &[WitFn]) -> Result<Vec<u8>> {
    let core = parse(data)?;
    let memory = core.memory.ok_or_else(|| anyhow!("TeaVM's module imports no memory"))?;
    let by_name: HashMap<(&str, &str), &WitFn> = wit_imports.iter().map(|w| ((w.module.as_str(), w.name.as_str()), w)).collect();
    let mut plan = Plan::default();
    let (i32, f64) = (ValType::I32, ValType::F64);

    // The world's imports first, with TeaVM's own types.
    let mut wit_targets = HashMap::new();
    for (index, &(module, name, ty)) in core.imports.iter().enumerate() {
        if let Some(w) = by_name.get(&(module, name)) {
            let (have, want) = (core.sig(ty)?, canonical(w));
            if *have != want {
                bail!("the app calls {module}#{name} as {}; the canonical ABI needs {}; rebuild to regenerate bindings/", show(have), show(&want));
            }
            wit_targets.insert(index, plan.import(module, name, ty));
        }
    }
    let wit = |plan: &mut Plan, (module, name): (&str, &str)| -> Result<u32> {
        let w = by_name.get(&(module, name)).ok_or_else(|| anyhow!("the world has no {module}#{name}"))?;
        let ty = plan.ty(&core, canonical(w));
        Ok(plan.import(module, name, ty))
    };
    let log = |level: &str| wit_imports.iter().find(|w| w.c_name == format!("appos_framework_log_{level}")).map(|w| (w.module.as_str(), w.name.as_str()));

    // Then TeaVM's runtime, each import in place by a shim of its type.
    let mut targets = Vec::new();
    for (index, &(module, name, ty)) in core.imports.iter().enumerate() {
        if let Some(&import) = wit_targets.get(&index) {
            targets.push(Target::Import(import));
            continue;
        }
        let sig = core.sig(ty)?.clone();
        let expect = |want: Sig| -> Result<()> {
            if sig != want {
                bail!("TeaVM's {module}#{name} is {}, expected {}", show(&sig), show(&want));
            }
            Ok(())
        };
        let array = |i: usize| match sig.0.get(i) {
            Some(ValType::Ref(r)) => match r.heap_type() {
                wasmparser::HeapType::Concrete(t) | wasmparser::HeapType::Exact(t) => {
                    t.as_module_index().ok_or_else(|| anyhow!("{module}#{name}: not an array"))
                }
                _ => Err(anyhow!("{module}#{name}: not an array")),
            },
            _ => Err(anyhow!("{module}#{name}: not an array")),
        };
        let shim = match (module, name) {
            ("wasm:js-string", _) => Shim::JsString(match name {
                "fromCharCode" => JsString::FromCharCode,
                "concat" => JsString::Concat,
                "substring" => JsString::Substring,
                "fromCharCodeArray" => JsString::FromCharCodeArray(array(0)?),
                "intoCharCodeArray" => JsString::IntoCharCodeArray(array(1)?),
                "length" => JsString::Length,
                "charCodeAt" => JsString::CharCodeAt,
                _ => bail!("TeaVM uses the JS string builtin {name}, which AppOS doesn't have yet"),
            }),
            ("teavmJso" | "teavm", _) | ("teavmMemory", "notifyHeapResized") => Shim::Empty,
            ("teavmDate", "currentTimeMillis") => {
                expect((vec![], vec![f64]))?;
                Shim::Clock { now: wit(&mut plan, WALL_CLOCK)?, scratch: plan.global(&core) }
            }
            ("teavmMath", "random") => {
                expect((vec![], vec![f64]))?;
                Shim::Random { random: wit(&mut plan, RANDOM)? }
            }
            ("teavmMath", _) => {
                let unary = |op: I<'static>| (vec![f64], vec![I::LocalGet(0), op]);
                let binary = |op: I<'static>| (vec![f64, f64], vec![I::LocalGet(0), I::LocalGet(1), op]);
                let (params, body) = match name {
                    "floor" => unary(I::F64Floor),
                    "ceil" => unary(I::F64Ceil),
                    "trunc" => unary(I::F64Trunc),
                    "sqrt" => unary(I::F64Sqrt),
                    "abs" => unary(I::F64Abs),
                    "round" => (vec![f64], vec![I::LocalGet(0), I::F64Const(0.5f64.into()), I::F64Add, I::F64Floor]),
                    "min" => binary(I::F64Min),
                    "max" => binary(I::F64Max),
                    _ => bail!("the app uses Math.{name}, which Java apps on AppOS don't have yet (TeaVM takes it from JavaScript)"),
                };
                expect((params, vec![f64]))?;
                Shim::Math(body)
            }
            ("teavmConsole", "putcharStdout" | "putcharStderr") => {
                expect((vec![i32], vec![]))?;
                let level = if name == "putcharStdout" { "info" } else { "warn" };
                let log = log(level).map(|n| wit(&mut plan, n)).transpose()?;
                Shim::Putchar { log, buf: plan.global(&core), len: plan.global(&core) }
            }
            _ => bail!("the app imports {module}#{name}, which neither the WIT nor AppOS's Java runtime has"),
        };
        plan.shims.push((shim, ty));
        targets.push(Target::Shim(plan.shims.len() - 1));
    }

    // Function indices: imports, then TeaVM's functions, then the shims.
    let imports = plan.imports.len() as u32;
    let shim_base = imports + core.funcs.len() as u32;
    let mut map: Vec<u32> = targets
        .iter()
        .map(|t| match t {
            Target::Import(i) => *i,
            Target::Shim(s) => shim_base + *s as u32,
        })
        .collect();
    map.extend((0..core.funcs.len() as u32).map(|j| imports + j));
    let mut remap = Remap(map);

    let defined = |name: &str| -> Result<(u32, &Sig)> {
        let index = core.export(name, ExternalKind::Func).ok_or_else(|| anyhow!("TeaVM's module has no export {name}; bindings/ should have it"))?;
        let local = (index as usize).checked_sub(core.imports.len()).ok_or_else(|| anyhow!("{name} is an import"))?;
        Ok((remap.0[index as usize], core.sig(core.funcs[local])?))
    };
    let (realloc, realloc_sig) = defined("cabi_realloc")?;
    if *realloc_sig != (vec![i32; 4], vec![i32]) {
        bail!("cabi_realloc is {}", show(realloc_sig));
    }
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("cabi_realloc", ExportKind::Func, realloc);
    for w in wit_exports {
        let (index, have) = defined(&w.name)?;
        let want = canonical(w);
        if *have != want {
            bail!("the app exports {} as {}; the canonical ABI needs {}; rebuild to regenerate bindings/", w.name, show(have), show(&want));
        }
        exports.export(&w.name, ExportKind::Func, index);
    }

    let chars = || core.chars.ok_or_else(|| anyhow!("TeaVM's module has no char[] type"));
    let mut bodies = Vec::new();
    for (shim, ty) in &plan.shims {
        let sig = core.sig(*ty)?;
        bodies.push(match shim {
            Shim::Empty => {
                let mut ins = Vec::new();
                for t in &sig.1 {
                    ins.extend(empty(&mut remap, *t)?);
                }
                body(&[], &ins)
            }
            Shim::JsString(s) => js_string(s, chars()?),
            Shim::Clock { now, scratch } => body(
                &[],
                &[
                    I::GlobalGet(*scratch),
                    I::I32Eqz,
                    I::If(BlockType::Empty),
                    I::I32Const(0),
                    I::I32Const(0),
                    I::I32Const(8),
                    I::I32Const(16),
                    I::Call(realloc),
                    I::GlobalSet(*scratch),
                    I::End,
                    I::GlobalGet(*scratch),
                    I::Call(*now),
                    // datetime: seconds (u64) at 0, nanoseconds (u32) at 8.
                    I::GlobalGet(*scratch),
                    I::I64Load(mem(0, 3)),
                    I::I64Const(1000),
                    I::I64Mul,
                    I::GlobalGet(*scratch),
                    I::I64Load32U(mem(8, 2)),
                    I::I64Const(1_000_000),
                    I::I64DivU,
                    I::I64Add,
                    I::F64ConvertI64U,
                ],
            ),
            // The top 53 bits as a double in [0, 1).
            Shim::Random { random } => {
                body(&[], &[I::Call(*random), I::I64Const(11), I::I64ShrU, I::F64ConvertI64U, I::F64Const((1.0f64 / (1u64 << 53) as f64).into()), I::F64Mul])
            }
            Shim::Math(ins) => body(&[], ins),
            Shim::Putchar { log: None, .. } => body(&[], &[]),
            Shim::Putchar { log: Some(log), buf, len } => {
                let flush = [
                    I::GlobalGet(*len),
                    I::If(BlockType::Empty),
                    I::GlobalGet(*buf),
                    I::GlobalGet(*len),
                    I::Call(*log),
                    I::I32Const(0),
                    I::GlobalSet(*len),
                    I::End,
                ];
                let mut ins = vec![I::LocalGet(0), I::I32Const(10), I::I32Eq, I::If(BlockType::Empty)];
                ins.extend(flush.clone());
                ins.extend([
                    I::Return,
                    I::End,
                    I::GlobalGet(*buf),
                    I::I32Eqz,
                    I::If(BlockType::Empty),
                    I::I32Const(0),
                    I::I32Const(0),
                    I::I32Const(2),
                    I::I32Const(LINE * 2),
                    I::Call(realloc),
                    I::GlobalSet(*buf),
                    I::End,
                    I::GlobalGet(*len),
                    I::I32Const(LINE),
                    I::I32Eq,
                    I::If(BlockType::Empty),
                ]);
                ins.extend(flush);
                ins.extend([
                    I::End,
                    I::GlobalGet(*buf),
                    I::GlobalGet(*len),
                    I::I32Const(1),
                    I::I32Shl,
                    I::I32Add,
                    I::LocalGet(0),
                    I::I32Store16(mem(0, 1)),
                    I::GlobalGet(*len),
                    I::I32Const(1),
                    I::I32Add,
                    I::GlobalSet(*len),
                ]);
                body(&[], &ins)
            }
        });
    }

    // TeaVM's globals: the heap after its data, as large as memory allows.
    let heap_offset = core.data_end.div_ceil(8) * 8;
    let mut globals = GlobalSection::new();
    for (module, name, ty) in &core.globals {
        let value = match (*module, *name) {
            ("teavmMemory", "heapOffset") => i32::try_from(heap_offset)?,
            ("teavmMemory", "maxSize") => i32::MAX,
            _ => bail!("the app imports the global {module}#{name}, which AppOS's Java runtime doesn't have"),
        };
        globals.global(remap.global_type(*ty)?, &ConstExpr::i32_const(value));
    }

    // Second pass: copy the module with the new indices, add the rest.
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut tables = None;
    let mut tags = None;
    let mut start = None;
    let mut elements = None;
    let mut data_count = None;
    let mut code = CodeSection::new();
    let mut data_section = None;
    for payload in Parser::new(0).parse_all(data) {
        match payload? {
            Payload::TypeSection(r) => remap.parse_type_section(&mut types, r)?,
            Payload::FunctionSection(r) => remap.parse_function_section(&mut functions, r)?,
            Payload::TableSection(r) => remap.parse_table_section(tables.get_or_insert_with(TableSection::new), r)?,
            Payload::TagSection(r) => remap.parse_tag_section(tags.get_or_insert_with(TagSection::new), r)?,
            Payload::GlobalSection(r) => remap.parse_global_section(&mut globals, r)?,
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
    for (params, results) in &plan.new_types {
        let encode = |t: &ValType| remap_val(*t);
        types.ty().function(params.iter().map(encode), results.iter().map(encode));
    }
    let mut import_section = ImportSection::new();
    for (module, name, ty) in &plan.imports {
        import_section.import(module, name, wasm_encoder::EntityType::Function(*ty));
    }
    for (_, ty) in &plan.shims {
        functions.function(*ty);
    }
    for f in &bodies {
        code.function(f);
    }
    for _ in 0..plan.new_globals {
        globals.global(GlobalType { val_type: wasm_encoder::ValType::I32, mutable: true, shared: false }, &ConstExpr::i32_const(0));
    }
    let mut memories = MemorySection::new();
    memories.memory(remap.memory_type(memory)?);

    let mut module = Module::new();
    module.section(&types);
    module.section(&import_section);
    module.section(&functions);
    if let Some(s) = &tables {
        module.section(s);
    }
    module.section(&memories);
    if let Some(s) = &tags {
        module.section(s);
    }
    module.section(&globals);
    module.section(&exports);
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
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all()).validate_all(&bytes).context("the rewritten module doesn't validate")?;
    Ok(bytes)
}

/// The JS string builtins over (array (mut i16)) `chars`.
fn js_string(s: &JsString, chars: u32) -> Function {
    let array = wasm_encoder::ValType::Ref(RefType { nullable: true, heap_type: HeapType::Concrete(chars) });
    let cast = |local: u32| [I::LocalGet(local), I::AnyConvertExtern, I::RefCastNonNull(HeapType::Concrete(chars))];
    let copy = |dst: u32, src: u32| I::ArrayCopy { array_type_index_dst: dst, array_type_index_src: src };
    let new_string = |local: u32| [I::LocalGet(local), I::RefAsNonNull, I::ExternConvertAny];
    let mut ins = Vec::new();
    let locals: &[wasm_encoder::ValType] = match s {
        JsString::FromCharCode => {
            ins.extend([I::LocalGet(0), I::ArrayNewFixed { array_type_index: chars, array_size: 1 }, I::ExternConvertAny]);
            &[]
        }
        JsString::Length => {
            ins.extend(cast(0));
            ins.push(I::ArrayLen);
            &[]
        }
        JsString::CharCodeAt => {
            ins.extend(cast(0));
            ins.extend([I::LocalGet(1), I::ArrayGetU(chars)]);
            &[]
        }
        // Locals: 2 a, 3 b, 4 result.
        JsString::Concat => {
            ins.extend(cast(0));
            ins.push(I::LocalSet(2));
            ins.extend(cast(1));
            ins.push(I::LocalSet(3));
            ins.extend([
                I::I32Const(0),
                I::LocalGet(2),
                I::ArrayLen,
                I::LocalGet(3),
                I::ArrayLen,
                I::I32Add,
                I::ArrayNew(chars),
                I::LocalSet(4),
                I::LocalGet(4),
                I::I32Const(0),
                I::LocalGet(2),
                I::I32Const(0),
                I::LocalGet(2),
                I::ArrayLen,
                copy(chars, chars),
                I::LocalGet(4),
                I::LocalGet(2),
                I::ArrayLen,
                I::LocalGet(3),
                I::I32Const(0),
                I::LocalGet(3),
                I::ArrayLen,
                copy(chars, chars),
            ]);
            ins.extend(new_string(4));
            &[array, array, array]
        }
        // (string, start, end); locals: 3 string, 4 result.
        JsString::Substring => {
            ins.extend(cast(0));
            ins.push(I::LocalSet(3));
            ins.extend([
                I::I32Const(0),
                I::LocalGet(2),
                I::LocalGet(1),
                I::I32Sub,
                I::ArrayNew(chars),
                I::LocalSet(4),
                I::LocalGet(4),
                I::I32Const(0),
                I::LocalGet(3),
                I::LocalGet(1),
                I::LocalGet(2),
                I::LocalGet(1),
                I::I32Sub,
                copy(chars, chars),
            ]);
            ins.extend(new_string(4));
            &[array, array]
        }
        // (char[], start, end); local 3 result.
        JsString::FromCharCodeArray(from) => {
            ins.extend([
                I::I32Const(0),
                I::LocalGet(2),
                I::LocalGet(1),
                I::I32Sub,
                I::ArrayNew(chars),
                I::LocalSet(3),
                I::LocalGet(3),
                I::I32Const(0),
                I::LocalGet(0),
                I::LocalGet(1),
                I::LocalGet(2),
                I::LocalGet(1),
                I::I32Sub,
                copy(chars, *from),
            ]);
            ins.extend(new_string(3));
            &[array]
        }
        // (string, char[], start) -> length; local 3 string.
        JsString::IntoCharCodeArray(into) => {
            ins.extend(cast(0));
            ins.extend([
                I::LocalSet(3),
                I::LocalGet(1),
                I::LocalGet(2),
                I::LocalGet(3),
                I::I32Const(0),
                I::LocalGet(3),
                I::ArrayLen,
                copy(*into, chars),
                I::LocalGet(3),
                I::ArrayLen,
            ]);
            &[array]
        }
    };
    body(locals, &ins)
}

/// An empty value of type `t`: zero, null, or for a non-null externref an
/// i31.
fn empty(remap: &mut Remap, t: ValType) -> Result<Vec<I<'static>>> {
    Ok(match t {
        ValType::I32 => vec![I::I32Const(0)],
        ValType::I64 => vec![I::I64Const(0)],
        ValType::F32 => vec![I::F32Const(0.0f32.into())],
        ValType::F64 => vec![I::F64Const(0.0f64.into())],
        ValType::Ref(r) if r.is_nullable() => vec![I::RefNull(remap.heap_type(r.heap_type())?)],
        ValType::Ref(r) if r.is_extern_ref() => vec![I::I32Const(0), I::RefI31, I::ExternConvertAny],
        _ => vec![I::Unreachable],
    })
}

fn remap_val(t: ValType) -> wasm_encoder::ValType {
    match t {
        ValType::I32 => wasm_encoder::ValType::I32,
        ValType::I64 => wasm_encoder::ValType::I64,
        ValType::F32 => wasm_encoder::ValType::F32,
        ValType::F64 => wasm_encoder::ValType::F64,
        other => unreachable!("imports the rewrite adds take only numbers, not {other:?}"),
    }
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg { offset, align, memory_index: 0 }
}

fn body(locals: &[wasm_encoder::ValType], ins: &[I]) -> Function {
    let mut f = Function::new(locals.iter().map(|t| (1, *t)));
    for i in ins {
        f.instruction(i);
    }
    f.instruction(&I::End);
    f
}
