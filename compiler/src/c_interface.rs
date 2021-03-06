// external C interface for the compiler (so that the language can use it)

use crate::common::*;
use crate::{lexer, parser};
use crate::compiler::Compiler;
use crate::expr::{Expr, ExprContent};

use std::fs::File;
use std::io::Read;
use std::ffi::CString;
use std::collections::HashMap;
use std::path::Path;
use std::fmt;
use std::mem::ManuallyDrop;
use std::time::{Instant, Duration};
use std::sync::mpsc::{channel, TryRecvError, Receiver};

use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent, ReadDirectoryChangesWatcher};
use libloading::{Library, Symbol};

use std::{thread, time};

/// A handle to a module
#[no_mangle]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct SModuleHandle {
  pub id : u64,
}

/// A generic option type is compatible with the runtime option representation
#[no_mangle]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct SOption<T : Copy + Clone> {
  pub is_some : bool,
  pub val : T,
}

impl <T : Copy + Clone> From<Option<T>> for SOption<T> {
  fn from(o : Option<T>) -> Self {
    if let Some(val) = o {
      SOption { is_some: true, val }
    }
    else {
      SOption { is_some: false, val: unsafe { std::mem::zeroed() } }
    }
  }
}

/// Owned array, compatible with the runtime array implementation
#[repr(C)]
pub struct SArray<T>(SSlice<T>);

impl <T> SArray<T> {
  pub fn new(mut v : Vec<T>) -> SArray<T> {
    let a = SArray(SSlice { length: v.len() as u64, data: v.as_mut_ptr() });
    std::mem::forget(v);
    a
  }

  pub fn as_slice(&self) -> &[T] {
    self.0.as_slice()
  }
}

impl <T : fmt::Debug> fmt::Debug for SArray<T> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{:?}", self.as_slice())
  }
}

impl <T> Drop for SArray<T> {
  fn drop(&mut self) {
    unsafe {
      Vec::from_raw_parts(self.0.data, self.0.length as usize, self.0.length as usize)
    };
  }
}

impl <T : Clone> Clone for SArray<T> {
  fn clone(&self) -> Self {
    let v = unsafe {
      Vec::from_raw_parts(self.0.data, self.0.length as usize, self.0.length as usize)
    };
    let a = SArray::new(v.clone());
    std::mem::forget(v);
    a
  }
}

/// A borrowed slice that is compatible with the runtime array representation
#[no_mangle]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct SSlice<T> {
  pub data : *mut T,
  pub length : u64,
}

impl <T> SSlice<T> {
  pub fn from_slice(s : &[T]) -> Self {
    let data = s.as_ptr() as *mut T;
    SSlice { data, length: s.len() as u64 }
  }

  pub fn as_slice(&self) -> &[T] {
    unsafe {
      std::slice::from_raw_parts(self.data, self.length as usize)
    }
  }
}

/// A sized string that is compatible with the runtime string representation
pub type SStr = SSlice<u8>;

impl SStr {
  pub fn from_str(s : &str) -> Self {
    let data = (s as *const str) as *mut u8;
    SStr { data, length: s.len() as u64 }
  }

  pub fn from_string(s : ManuallyDrop<String>) -> Self {
    Self::from_str(&s)
  }

  pub fn as_str(&self) -> &str {
    unsafe {
      let slice = std::slice::from_raw_parts(self.data, self.length as usize);
      std::str::from_utf8_unchecked(slice)
    }
  }
}

impl fmt::Debug for SStr {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.as_str())
  }
}

#[cfg(not(debug_assertions))]
static MODE : &'static str = "release";
#[cfg(debug_assertions)]
static MODE : &'static str = "debug";

#[cfg(not(test))]
static ROOT : &'static str = "";
#[cfg(test)]
static ROOT : &'static str = "../";

#[no_mangle]
static TEST_GLOBAL : i64 = 47;

extern {
  pub fn malloc(size: usize) -> *mut u8;
  pub fn free(ptr: *mut u8);
  pub fn memcpy(dest : *mut u8, src: *const u8, count : usize) -> *mut u8;
}

#[no_mangle]
pub extern "C" fn panic(s : SStr) {
  panic!("EXPLICIT PANIC: {}", s.as_str())
}

#[no_mangle]
pub extern "C" fn load_expression(c : *mut Compiler, code_path : SStr) -> Box<Expr> {
  let mut f = File::open(code_path.as_str()).unwrap_or_else(|_| panic!("load_expression failed. file '{}' not found", code_path.as_str()));
  let mut code = String::new();
  f.read_to_string(&mut code).unwrap();
  let c = unsafe { &mut *c };
  let aaa = (); // TODO: this is wrong. Use the code store to do this, so that the source id is logged properly.
  let tokens = lexer::lex(no_source(), &code, &c.cache).unwrap();
  let expr = parser::parse(no_source(), tokens, &c.cache).unwrap();
  Box::new(expr)
}

#[no_mangle]
pub extern "C" fn get_module(c : *mut Compiler, name : SStr, unit_id_out : &mut SOption<UnitId>) {
  let c = unsafe { &mut *c };
  let name = name.as_str();
  *unit_id_out = c.code_store.named_unit(name).into();
}

#[no_mangle]
pub extern "C" fn load_module(c : *mut Compiler, maybe_name : SStr, imports : SSlice<UnitId>, e : &Expr, out : &mut SOption<UnitId>) {
  let c = unsafe { &mut *c };
  let imports = imports.as_slice();
  let maybe_name = maybe_name.as_str();
  let name = if maybe_name == "" { None } else { Some(maybe_name) };
  *out = match c.load_expr_as_module(e, name, imports) {
    Ok((unit_id, _val)) => Some(unit_id).into(),
    Err(_e) => {
      println!("Failed to load module");
      None.into()
    }
  };
}

pub extern "C" fn unload_module(c : *mut Compiler, unit_id : UnitId) {
  let c = unsafe { &mut *c };
  c.code_store.remove_unit(unit_id);
}

pub extern "C" fn find_all_dependents(c : *mut Compiler, unit_id : UnitId, out : &mut SArray<UnitId>) {
  let c = unsafe { &mut *c };
  let deps = c.find_all_dependents(unit_id);
  *out = SArray::new(deps);
}

// TODO: panics if there is more than one overload, because no argument types
// are provided to narrow the search, and it would be very unsafe to return
// the wrong one.
#[no_mangle]
pub extern "C" fn get_function(
  c : *mut Compiler,
  unit_id : UnitId,
  name : SStr,
  out : &mut SOption<*mut u8>
)
{
  let c = unsafe { &mut *c };
  let types = c.code_store.types(unit_id);
  let name = name.as_str();
  let mut i = types.symbols.values()
    .filter(|def| def.name.as_ref() == name && def.type_tag.sig().is_some())
    .flat_map(|def| def.codegen_name());
  let lu = c.code_store.llvm_unit(unit_id);
  let address =
    i.next().and_then(|codegen_name|
      unsafe { lu.ee.get_function_address(codegen_name) });
  *out = if i.next().is_some() {
    println!("two matching overloads for '{}' in get_function_address", name);
    None.into()
  }
  else {
    address.map(|u| u as *mut u8).into()
  };
}

//out : &mut SOption<UnitId>

#[no_mangle]
pub extern "C" fn template_quote(e : &Expr, args : SSlice<&Expr>) -> Box<Expr> {
  fn template(e : &Expr, args : &[&Expr], next_arg : &mut usize) -> Expr {
    if let Some((name, es)) = e.try_construct() {
      match name {
        "$" => {
          let new_e = args[*next_arg].clone();
          *next_arg += 1;
          new_e
        }
        _ => {
          let mut children = vec![];
          for expr in es {
            children.push(template(expr, args, next_arg));
          }
          let loc = e.loc;
          let content = ExprContent::list(name.into(), children);
          Expr { loc, content }
        }
      }
    }
    else {
      e.clone()
    }
  }
  Box::new(template(e, args.as_slice(), &mut 0))
}

#[no_mangle]
pub extern "C" fn print_string(s : SStr) {
  print!("{}", s.as_str());
}

pub type TimerHandle = ManuallyDrop<Box<Instant>>;

#[no_mangle]
pub extern "C" fn start_timer() -> TimerHandle {
  ManuallyDrop::new(Box::new(Instant::now()))
}

#[no_mangle]
pub extern "C" fn drop_timer(t : TimerHandle) {
  ManuallyDrop::into_inner(t);
}

#[no_mangle]
pub extern "C" fn millis_elapsed(timer : TimerHandle) -> u64 {
  let v = Instant::now();
  v.duration_since(**timer).as_millis() as u64
}

pub struct FileWatcher {
  watcher : ReadDirectoryChangesWatcher,
  rx : Receiver<DebouncedEvent>,
}

pub type WatcherHandle = ManuallyDrop<Box<FileWatcher>>;

#[no_mangle]
pub extern "C" fn poll_watcher_event(w : WatcherHandle, path_out : &mut SOption<SStr>) {
  let out = match w.rx.try_recv() {
    Ok(event) => {
      match event {
        DebouncedEvent::Write(path) => {
          let path : String = path.to_str().unwrap().replace("\\", "/");
          Some(SStr::from_string(ManuallyDrop::new(path)))
        }
        _ => None,
      }
    },
    Err(e) => match e {
      TryRecvError::Disconnected => None,
      TryRecvError::Empty => None,
    },
  };
  *path_out = out.into();
}

#[no_mangle]
pub extern "C" fn create_watcher(millisecond_interval : u64) -> WatcherHandle {
  let (tx, rx) = channel();
  let watcher = watcher(tx, Duration::from_millis(millisecond_interval)).unwrap();
  ManuallyDrop::new(Box::new(FileWatcher { watcher, rx}))
}

#[no_mangle]
pub extern "C" fn drop_watcher(w : WatcherHandle) {
  ManuallyDrop::into_inner(w);
}

#[no_mangle]
pub extern "C" fn watch_file(mut w : WatcherHandle, path : SStr) {
  if w.watcher.watch(path.as_str(), RecursiveMode::Recursive).is_err() {
    panic!("failed to watch file '{}'", path.as_str())
  }
}

use rand::{Rng, SeedableRng, rngs::SmallRng};

pub type RNGHandle = ManuallyDrop<Box<SmallRng>>;

#[no_mangle]
pub extern "C" fn seeded_rng(seed : u64) -> RNGHandle {
  ManuallyDrop::new(Box::new(SmallRng::seed_from_u64(seed)))
}

#[no_mangle]
pub extern "C" fn drop_seeded_rng(rng : RNGHandle) {
  ManuallyDrop::into_inner(rng);
}

#[no_mangle]
pub extern "C" fn rand_f64(mut rng : RNGHandle) -> f64 {
  rng.gen()
}

#[no_mangle]
pub extern "C" fn rand_u64(mut rng : RNGHandle) -> u64 {
  rng.gen()
}

pub extern "C" fn print_type<T : std::fmt::Display>(t : T) {
  print!("{}", t);
}

#[no_mangle]
pub extern "C" fn print_expr(e : &Expr) {
  println!("{}", e);
}

#[no_mangle]
pub extern "C" fn expr_to_string(out : &mut SStr, e : &Expr) {
  let string = format!("{}", e);
  let s = SStr::from_str(string.as_str());
  std::mem::forget(string);
  *out = s;
}

/// defined for the test suite only
#[no_mangle]
pub extern "C" fn test_add(a : i64, b : i64) -> i64 {
  a + b
}

#[no_mangle]
pub extern "C" fn thread_sleep(millis : u64) {
  let t = time::Duration::from_millis(millis);
  thread::sleep(t);
}

#[no_mangle]
pub extern "C" fn load_library_c(lib_name : SStr) -> usize {
  let lib = lib_name.as_str();
  let deps_path = format!("{}target/{}/deps/{}.dll", ROOT, MODE, lib);
  let local_path = format!("{}.dll", lib);
  let paths = [deps_path.as_str(), local_path.as_str()];
  paths.iter().cloned().flat_map(load_library).nth(0).unwrap_or(0)
}

static mut SHARED_LIBRARIES : Option<HashMap<usize, (RefStr, Library)>> = None;
static mut SHARED_LIB_HANDLE_COUNTER : usize = 0;

/// TODO: This is not thread-safe!
pub fn load_library(path : &str) -> Option<usize> {
  let path = Path::new(path);
  let file_name = path.file_name().unwrap().to_str().unwrap();
  let r = Library::new(path);
  if r.is_err() {
    return None;
  }
  let lib = r.unwrap();
  unsafe {
    if SHARED_LIBRARIES.is_none() {
      SHARED_LIBRARIES = Some(HashMap::new());
    }
    SHARED_LIB_HANDLE_COUNTER += 1;
    let handle = SHARED_LIB_HANDLE_COUNTER;
    SHARED_LIBRARIES.as_mut().unwrap().insert(handle, (file_name.into(), lib));
    Some(handle)
  }
}

/// TODO: This is not thread-safe!
#[no_mangle]
pub extern "C" fn load_symbol(lib_handle : usize, symbol_name : SStr) -> usize {
  let s = CString::new(symbol_name.as_str()).unwrap();
  unsafe {
    if SHARED_LIBRARIES.is_none() {
      panic!();
    }
    let (_, lib) = SHARED_LIBRARIES.as_ref().unwrap().get(&lib_handle).unwrap();
    let symbol: Option<Symbol<*const ()>> =
      lib.get(s.as_bytes_with_nul()).ok();
    symbol.map(|sym| sym.into_raw().into_raw() as usize).unwrap_or(0)
  }
}

pub struct CSymbols {
  pub local_symbol_table : HashMap<RefStr, usize>,
}

impl CSymbols {
  pub fn new_populated() -> CSymbols {
    let mut cs = CSymbols {
      local_symbol_table: HashMap::new(),
    };
    cs.populate();
    cs
  }

  fn populate(&mut self) {
    let sym = &mut self.local_symbol_table;
    sym.insert("load_library".into(), (load_library_c as *const()) as usize);
    sym.insert("load_symbol".into(), (load_symbol as *const()) as usize);
    sym.insert("malloc64".into(), (malloc as *const()) as usize);
    sym.insert("free".into(), (free as *const()) as usize);
    sym.insert("memcpy".into(), (memcpy as *const()) as usize);
    sym.insert("panic".into(), (panic as *const()) as usize);
    

    sym.insert("print_string".into(), (print_string as *const()) as usize);
    sym.insert("print_expr".into(), (print_expr as *const()) as usize);
    sym.insert("print_i64".into(), (print_type::<i64> as *const()) as usize);
    sym.insert("print_u64".into(), (print_type::<u64> as *const()) as usize);
    sym.insert("print_f64".into(), (print_type::<f64> as *const()) as usize);
    sym.insert("print_bool".into(), (print_type::<bool> as *const()) as usize);

    sym.insert("template_quote".into(), (template_quote as *const()) as usize);
    sym.insert("thread_sleep".into(), (thread_sleep as *const()) as usize);

    sym.insert("expr_to_string".into(), (expr_to_string as *const()) as usize);

    sym.insert("load_expression".into(), (load_expression as *const()) as usize);
    sym.insert("load_module".into(), (load_module as *const()) as usize);
    sym.insert("unload_module".into(), (unload_module as *const()) as usize);
    sym.insert("find_all_dependents".into(), (find_all_dependents as *const()) as usize);
    sym.insert("get_module".into(), (get_module as *const()) as usize);
    sym.insert("get_function".into(), (get_function as *const()) as usize);

    sym.insert("start_timer".into(), (start_timer as *const()) as usize);
    sym.insert("drop_timer".into(), (drop_timer as *const()) as usize);
    sym.insert("millis_elapsed".into(), (millis_elapsed as *const()) as usize);

    sym.insert("poll_watcher_event".into(), (poll_watcher_event as *const()) as usize);
    sym.insert("create_watcher".into(), (create_watcher as *const()) as usize);
    sym.insert("drop_watcher".into(), (drop_watcher as *const()) as usize);
    sym.insert("watch_file".into(), (watch_file as *const()) as usize);

    sym.insert("seeded_rng".into(), (seeded_rng as *const()) as usize);
    sym.insert("drop_seeded_rng".into(), (drop_seeded_rng as *const()) as usize);
    sym.insert("rand_f64".into(), (rand_f64 as *const()) as usize);
    sym.insert("rand_u64".into(), (rand_u64 as *const()) as usize);

    sym.insert("test_add".into(), (test_add as *const()) as usize);
    sym.insert("test_global".into(), (&TEST_GLOBAL as *const i64) as usize);
  }

  pub fn add_symbol<T>(&mut self, name : &str, p : *mut T) {
    // This is a bit confusing. When we link to a global we do it by passing a
    // pointer. Since this global *is* a pointer, we have to pass a pointer to
    // a pointer, which requires another permanent heap allocation for indirection.
    // TODO: can this be improved?
    let p = Box::into_raw(Box::new(p));
    self.local_symbol_table.insert(name.into(), p as usize);
  }
}
