
use crate::{
  error, expr, c_interface, llvm_compile, types, code_store,
  structure, lexer, parser, inference_solver
};
use expr::{StringCache, Expr, UIDGenerator};
use c_interface::CSymbols;
use code_store::{CodeStore, SourceId, PolyFunction};
use types::{TypeContent, PType, UnitId};
use llvm_compile::{LlvmCompiler, execute_function};
use error::{Error, error};
use structure::TOP_LEVEL_FUNCTION_NAME;

use std::collections::{HashMap, VecDeque, HashSet};

// TODO: Put these options somewhere more sensible
pub static DEBUG_PRINTING_EXPRS : bool = false;
pub static DEBUG_PRINTING_IR : bool = false;
pub static ENABLE_IR_OPTIMISATION : bool = false;

pub struct Compiler {
  pub code_store : CodeStore,
  pub llvm_compiler : LlvmCompiler,
  pub gen : UIDGenerator,
  pub cache : StringCache,
  pub c_symbols : CSymbols,
}

impl Compiler {
  pub fn new() -> Box<Compiler> {
    let mut gen = UIDGenerator::new();
    let cache = StringCache::new();
    let code_store  = CodeStore::new_with_intrinsics(&mut gen, &cache);
    let llvm_compiler = LlvmCompiler::new();
    let c_symbols = CSymbols::new_populated();
    let mut c = Box::new(Compiler { 
      code_store, llvm_compiler, gen, cache, c_symbols
    });
    let cptr = (&mut *c) as *mut Compiler;
    c.c_symbols.add_symbol("compiler", cptr);
    c
  }

  pub fn load_expr_as_module(&mut self, expr : &Expr)
    -> Result<(UnitId, Val), Error>
  {
    let unit_id = self.gen.next().into();
    self.code_store.exprs.insert(unit_id, expr.clone());
    self.load_module_from_expr_internal(unit_id)?;
    let val = self.code_store.vals.get(&unit_id).unwrap().clone();
    Ok((unit_id, val))
  }

  pub fn load_module(&mut self, code : &str)
    -> Result<(UnitId, Val), Error>
  {
    let source_id = self.gen.next().into();
    self.code_store.code.insert(source_id, code.into());
    let unit_id = self.gen.next().into();
    self.parse(source_id, unit_id)?;
    self.load_module_from_expr_internal(unit_id)?;
    let val = self.code_store.vals.get(&unit_id).unwrap().clone();
    Ok((unit_id, val))
  }

  fn parse(&mut self, source_id : SourceId, unit_id : UnitId) -> Result<(), Error> {
    let code = self.code_store.code.get(&source_id).unwrap();
    let tokens =
      lexer::lex(&code, &self.cache)
      .map_err(|mut es| es.remove(0))?;
    let expr = parser::parse(tokens, &self.cache)?;
    self.code_store.exprs.insert(unit_id, expr);
    Ok(())
  }

  fn load_module_from_expr_internal(&mut self, unit_id : UnitId) -> Result<(), Error> {
    self.structure(unit_id)?;
    self.typecheck(unit_id)?;
    self.codegen(unit_id)?;
    self.initialise(unit_id)?;
    Ok(())
  }

  fn structure(&mut self, unit_id : UnitId) -> Result<(), Error> {
    let expr = self.code_store.exprs.get(&unit_id).unwrap();
    let nodes = structure::to_nodes(&mut self.gen, &self.cache, &expr)?;
    self.code_store.nodes.insert(unit_id, nodes);
    Ok(())
  }

  fn typecheck(&mut self, unit_id : UnitId) -> Result<(), Error> {
    let (types, mapping) =
      inference_solver::infer_types(
        unit_id, &self.code_store, &self.cache, &mut self.gen)?;
    for def in types.symbols.values() {
      if def.is_polymorphic() {
        let pf = PolyFunction {
          source_unit: def.unit_id,
          instances: HashMap::new(),
        };
        self.code_store.poly_functions.insert(def.id, pf);
      }
    }
    self.code_store.types.insert(unit_id, types);
    self.code_store.type_mappings.insert(unit_id, mapping);
    self.typecheck_new_polymorphic_instances(unit_id)?;
    Ok(())
  }

  fn typecheck_new_polymorphic_instances(&mut self, unit_id : UnitId) -> Result<(), Error> {
    // Typecheck and codegen any new polymorphic function instances
    let mut new_types = vec![];
    let mut polymorph_search_queue = VecDeque::new();
    polymorph_search_queue.push_back(unit_id);
    while let Some(psid) = polymorph_search_queue.pop_front() {
      let mapping = self.code_store.type_mappings.get(&psid).unwrap();
      for (poly_unit_id, symbol_id, type_tag) in mapping.polymorphic_references.iter() {
        if let Some(pf) = self.code_store.poly_functions.get(symbol_id) {
          if !pf.instances.contains_key(type_tag) {
            // Create a new unit for the function instance and typecheck it
            let instance_unit_id = self.gen.next().into();
            let poly_def = self.code_store.types(*poly_unit_id).symbols.get(symbol_id).unwrap();
            let (instance_types, instance_mapping, instance_symbol_id) =
              inference_solver::typecheck_polymorphic_function_instance(
                instance_unit_id, poly_def, type_tag, &self.code_store, &self.cache, &mut self.gen)?;
            // Register the instance with the code store
            let pf = self.code_store.poly_functions.get_mut(symbol_id).unwrap();
            pf.instances.insert(type_tag.clone(), (instance_unit_id, instance_symbol_id));
            new_types.push((instance_unit_id, instance_types, instance_mapping));
            // Register the new unit to be searched for more polymorphic instances
            polymorph_search_queue.push_back(instance_unit_id);
          }
        }
      }
      // Register new type info with the code store
      for (instance_unit_id, instance_types, instance_mapping) in new_types.drain(..) {
        self.code_store.types.insert(instance_unit_id, instance_types);
        self.code_store.type_mappings.insert(instance_unit_id, instance_mapping);
      }
    }
    Ok(())
  }

  fn codegen(&mut self, unit_id : UnitId) -> Result<(), Error> {
    // Find all of the polymorphic instances that this unit depends on,
    // and which still need to be code generated
    let mut units_to_codegen = HashSet::new();
    units_to_codegen.insert(unit_id);
    let mut polymorph_search_queue = VecDeque::new();
    polymorph_search_queue.push_back(unit_id);
    while let Some(psid) = polymorph_search_queue.pop_front() {
      let mapping = self.code_store.type_mappings.get(&psid).unwrap();
      for (_, symbol_id, type_tag) in mapping.polymorphic_references.iter() {
        let pf = self.code_store.poly_functions.get(symbol_id).unwrap();
        let instance_unit_id = pf.instances.get(type_tag).unwrap().0;
        if !self.code_store.llvm_units.contains_key(&instance_unit_id) {
          units_to_codegen.insert(instance_unit_id);
          polymorph_search_queue.push_back(instance_unit_id);
        }
      }
    }
    // Codegen the new units
    for &id in units_to_codegen.iter() {
      let lu = self.llvm_compiler.compile_unit(id, &self.code_store)?;
      self.code_store.llvm_units.insert(id, lu);
    }
    // Link the new units
    for &id in units_to_codegen.iter() {
      llvm_compile::link_unit(id, &self.code_store, &self.c_symbols);
    }
    Ok(())
  }

  fn initialise(&mut self, unit_id : UnitId) -> Result<(), Error> {
    let val = self.run_top_level(unit_id)?;
    self.code_store.vals.insert(unit_id, val);
    Ok(())
  }

  fn run_top_level(&self, unit_id : UnitId) -> Result<Val, Error> {
    use TypeContent::*;
    use PType::*;
    let f = TOP_LEVEL_FUNCTION_NAME;
    let types = self.code_store.types(unit_id);
    let def = types.symbols.values().find(|def| def.name.as_ref() == f).unwrap();
    let f = def.codegen_name().unwrap();
    let sig = if let Some(sig) = def.type_tag.sig() {sig} else {panic!()};
    let lu = self.code_store.llvm_unit(unit_id);
    let value = match &sig.return_type.content {
      Prim(Bool) => Val::Bool(execute_function(f, lu)),
      Prim(F64) => Val::F64(execute_function(f, lu)),
      Prim(F32) => Val::F32(execute_function(f, lu)),
      Prim(I64) => Val::I64(execute_function(f, lu)),
      Prim(I32) => Val::I32(execute_function(f, lu)),
      Prim(U64) => Val::U64(execute_function(f, lu)),
      Prim(U32) => Val::U32(execute_function(f, lu)),
      Prim(U16) => Val::U16(execute_function(f, lu)),
      Prim(U8) => Val::U8(execute_function(f, lu)),
      Prim(Void) => {
        execute_function::<()>(f, lu);
        Val::Void
      }
      t => {
        let loc = self.code_store.nodes(unit_id).root().loc;
        return error(loc, format!("can't return value of type {:?} from a top-level function", t));
      }
    };
    Ok(value)
  }

}

pub fn run_program(code : &str) -> Result<Val, Error> {
  let mut c = Compiler::new();
  let (_, val) = c.load_module(code)?;
  Ok(val)
}

#[derive(Clone, PartialEq, Debug)]
pub enum Val {
  Void,
  F64(f64),
  F32(f32),
  I64(i64),
  U64(u64),
  I32(i32),
  U32(u32),
  U16(u16),
  U8(u8),
  String(String),
  Bool(bool),
}
