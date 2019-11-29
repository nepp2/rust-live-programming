
use std::fmt;
use itertools::Itertools;

use crate::error::{Error, error, error_raw, TextLocation};
use crate::expr::{Expr, UIDGenerator};
use crate::structure::{
  Node, NodeId, Nodes, Symbol as RefSymbol, SymbolId, Content,
  Val, LabelId, TypeKind, FunctionNode, VarScope,
  GlobalType,
};
use crate::types::{
  Type, PType, TypeInfo, TypeDefinition, ConcreteGlobal,
  FunctionSignature, FunctionInit, GlobalDefinition,
  TypeDirectory, GlobalInit, Symbol, unify_abstract,
  AbstractType,
};
use crate::modules::TypedModule;
use crate::arena::{ Arena, Ap };

use std::collections::HashMap;

pub fn infer_types(
  nodes : Nodes,
  imports : &[&TypeInfo],
  gen : &mut UIDGenerator,
)
  -> Result<TypedModule, Vec<Error>>
{
  let arena = Arena::new();
  let mut c = Constraints::new();
  let mut cg = CodegenInfo::new();
  let mut errors = vec![];
  let module_id = gen.next().into();
  let mut new_module = TypeInfo::new(module_id);
  let mut type_directory =
  TypeDirectory::new(module_id, imports, &mut new_module);
  gather_constraints(
    &arena, &mut type_directory, &mut cg,
    gen, &mut c, &mut errors, &nodes);
  let mut i = 
    Inference::new(
      &arena, &nodes, &mut type_directory,
      &mut cg, &c, gen, &mut errors);
  i.infer();
  if errors.len() > 0 {
    Err(errors)
  }
  else {
    Ok(TypedModule::new(arena, module_id, nodes, new_module, cg))
  }
}

pub struct CodegenInfo {
  pub node_type : HashMap<NodeId, Type>,
  pub sizeof_info : HashMap<NodeId, Type>,
  pub symbol_references : HashMap<NodeId, Ap<GlobalDefinition>>,
  pub type_def_references : HashMap<Ap<str>, Ap<TypeDefinition>>,
}

impl CodegenInfo {
  pub fn new() -> Self {
    CodegenInfo {
      node_type: HashMap::new(),
      sizeof_info: HashMap::new(),
      symbol_references: HashMap::new(),
      type_def_references: HashMap::new(),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TypeClass {
  Float,
  Integer,
}

impl TypeClass {
  fn contains_type(self, t : Type) -> bool {
    match self {
      TypeClass::Float => t.float(),
      TypeClass::Integer => t.int(),
    }
  }

  fn default_type(self) -> Option<Type> {
    match self {
      TypeClass::Float => Some(Type::Prim(PType::F64)),
      TypeClass::Integer => Some(Type::Prim(PType::I64)),
    }
  }
}

struct Inference<'a> {
  arena : &'a Arena,
  nodes : &'a Nodes,
  t : &'a mut TypeDirectory<'a>,
  cg : &'a mut CodegenInfo,
  c : &'a Constraints,
  gen : &'a mut UIDGenerator,
  errors : &'a mut Vec<Error>,
  resolved : HashMap<TypeSymbol, Type>,
}

impl <'a> Inference<'a> {

  fn new(
    arena : &'a Arena,
    nodes : &'a Nodes,
    t : &'a mut TypeDirectory<'a>,
    cg : &'a mut CodegenInfo,
    c : &'a Constraints,
    gen : &'a mut UIDGenerator,
    errors : &'a mut Vec<Error>)
      -> Self
  {
    Inference {
      arena, nodes, t, cg, c, gen, errors,
      resolved: HashMap::new(),
    }
  }

  fn get_type(&self, ts : TypeSymbol) -> Option<Type> {
    self.resolved.get(&ts).cloned()
  }

  fn set_type(&mut self, ts : TypeSymbol, t : Type) {
    if let Some(prev_t) = self.resolved.get(&ts).cloned() {
      if let Some(unified_type) = unify_abstract(prev_t, t) {
        let aaa = (); // TODO: This needs to trigger re-evaluation of other constraints
        self.resolved.insert(ts, unified_type);
      }
      else {
        let e = error_raw(self.loc(ts),
          format!("conflicting types inferred; {} and {}.", t, prev_t));
        self.errors.push(e);
      }
    }
    else {
      self.resolved.insert(ts, t);
    }
  }

  fn loc(&self, ts : TypeSymbol) -> TextLocation {
    *self.c.symbols.get(&ts).unwrap()
  }

  fn unresolved_constraint_error(&mut self, c : &Constraint) {
    let e = match c  {
      Constraint::Assert(_ts, _t) => panic!(),
      Constraint::Equalivalent(_a, _b) => return,
      Constraint::FunctionDef{ name, loc, args, .. } => {
        error_raw(loc,
          format!("function definition '{}({})' not resolved", name,
            args.iter().map(|(s, ts)| {
              let t = self.get_type(*ts)
                .map(|t| format!("{}", t))
                .unwrap_or_else(|| "???".into());
              format!("{} : {}", s.name, t)
            }).join(", ")))
      }
      Constraint::FunctionCall{ node:_, function, args, result, mut_sig:_ } => {
        let loc = self.loc(*result);
        if let Function::Name(sym) = function {
          let unknown = Type::Abstract(AbstractType::Any);
          let arg_types : Vec<_> =
            args.iter().map(|(_, ts)| self.get_type(*ts).unwrap_or(unknown)).collect();
          let symbols = self.t.find_global(&sym.name, unknown, self.arena, self.gen);
          error_raw(loc,
            format!("function call {}({}) not resolved.\n   Symbols available:\n{}",
              sym.name,
              arg_types.iter().join(", "),
              symbols.iter()
                .map(|g| format!("      {} : {}", g.def.name, g.concrete_type))
                .join("\n")))
        }
        else {
          error_raw(loc, "function call not resolved")
        }
      }
      Constraint::Constructor { type_name, fields:_, result } => {
        error_raw(self.loc(*result),
          format!("constructor for '{}' not resolved", type_name))
      }
      Constraint::Convert { val, into_type } => {
        let unknown = Type::Abstract(AbstractType::Any);
        let t = self.get_type(*val).unwrap_or(unknown);
        let s = format!("type conversion from {} into {} not supported", t, into_type);
        error_raw(self.loc(*val), s)
      }
      Constraint::GlobalDef { name, type_symbol:_, initialiser:_, loc } => {
        error_raw(loc,
          format!("global definition '{}' not resolved", name))
      }
      Constraint::GlobalReference { node:_, name, result } => {
        error_raw(self.loc(*result),
          format!("global reference '{}' not resolved", name))
      }
      Constraint::FieldAccess{ container:_, field, result:_ } => {
        error_raw(field.loc,
          format!("field access '{}' not resolved", field.name))
      }
      Constraint::Array{ array, element:_ } => {
        error_raw(self.loc(*array), "array literal not resolved")
      }
    };
    self.errors.push(e);
  }

  fn register_def(&mut self, node : NodeId, def : Ap<GlobalDefinition>) {
    self.cg.symbol_references.insert(node, def);
  }

  fn find_global(&mut self, name : &str, t : Type)
    -> Option<Result<ConcreteGlobal, ()>> 
  {
    match self.t.find_global(name, t, self.arena, self.gen) {
      [g] => Some(Ok(*g)),
      _ => None,
    }
  }

  fn process_constraint(&mut self, c : &Constraint) -> bool {
    match c  {
      Constraint::Assert(ts, t) => {
        self.set_type(*ts, *t);
        return true;
      }
      Constraint::Equalivalent(a, b) => {
        if let Some(t) = self.get_type(*a) {
          if t.is_concrete() {
            self.set_type(*b, t);
            return true;
          }
        }
        if let Some(t) = self.get_type(*b) {
          if t.is_concrete() {
            self.set_type(*a, t);
            return true;
          }
        }
      }
      Constraint::FunctionDef{ name, return_type, args, body, loc } => {
        let resolved_args_count = args.iter().flat_map(|(_, ts)| self.get_type(*ts)).count();
        let return_type = self.get_type(*return_type);
        if resolved_args_count == args.len() && return_type.is_some() {
          let mut arg_names = vec!();
          let mut arg_types = vec!();
          for (arg, arg_ts) in args.iter() {
            arg_names.push(arg.clone());
            arg_types.push(self.get_type(*arg_ts).unwrap());
          }
          let sig = FunctionSignature {
            return_type: return_type.unwrap(),
            args: self.arena.alloc_slice(arg_types.as_slice()),
          };
          let name_for_codegen =
            self.arena.alloc_str(format!("{}.{}", name, self.gen.next()).as_str());
          let f = FunctionInit {
            body: *body,
            name_for_codegen,
            args: self.arena.alloc_slice(arg_names.as_slice()),
          };
          let g = GlobalDefinition {
            module_id: self.t.new_module_id(),
            name: self.arena.alloc_str(name),
            type_tag: Type::Fun(self.arena.alloc(sig)),
            initialiser: GlobalInit::Function(self.arena.alloc(f)),
            loc: *loc,
          };
          self.t.create_global(self.arena.alloc(g));
          return true;
        }
      }
      Constraint::FunctionCall{ node, function, args, result, mut_sig } => {
        let unknown = Type::Abstract(AbstractType::Any);
        let mut mut_args = Ap::get_mut(mut_sig.args);
        for (i, (_, ts)) in args.iter().enumerate() {
          mut_args[i] = self.get_type(*ts).unwrap_or(unknown);
        }
        Ap::get_mut(*mut_sig).return_type =
        self.get_type(*result).unwrap_or(unknown); 
        match function {
          Function::Name(sym) => {
            if let Some(r) = self.find_global(&sym.name, Type::Fun(*mut_sig)) {
              if let Ok(g) = r {
                self.register_def(*node, g.def);
                self.set_type(*result, g.concrete_type.signature().unwrap().return_type);
              }
              return true;
            }
          }
          Function::Value(ts) => {
            if let Some(t) = self.get_type(*ts) {
              if let Type::Fun(sig) = t {
                let rt = sig.return_type;
                self.set_type(*result, rt);
              }
              else {
                let e = error_raw(self.loc(*ts), "cannot call value of this type as function");
                self.errors.push(e);
              }
              return true;
            }
          }
        }
      }
      Constraint::Constructor { type_name, fields, result } => {
        if let Some(def) = self.t.find_type_def(type_name) {
          match def.kind {
            TypeKind::Struct => {
              if fields.len() == def.fields.len() {
                let it = fields.iter().zip(def.fields.iter());
                let mut arg_types = vec![];
                for ((field_name, _), (expected_name, expected_type)) in it {
                  if let Some(field_name) = field_name {
                    if field_name.name != expected_name.name {
                      self.errors.push(error_raw(field_name.loc, "incorrect field name"));
                    }
                  }
                  arg_types.push(*expected_type);
                }
                for((_, ts), t) in fields.iter().zip(arg_types.iter()) {
                  self.set_type(*ts, *t);
                }
              }
              else{
                let e = error_raw(self.loc(*result), "incorrect number of field arguments for struct");
                self.errors.push(e);
              }
            }
            TypeKind::Union => {
              if let [(Some(sym), ts)] = fields.as_slice() {
                if let Some((_, t)) = def.fields.iter().find(|(n, _)| n.name == sym.name) {
                  let t = *t;
                  self.set_type(*ts, t);
                }
                else {
                  self.errors.push(error_raw(sym.loc, "field does not exist in this union"));
                }
              }
              else {
                let s = format!("incorrect number of field arguments for union '{}'", type_name);
                let e = error_raw(self.loc(*result), s);
                self.errors.push(e);
              }
            }
          }
          let def_name = self.arena.alloc_str(type_name);
          self.set_type(*result, Type::Def(def_name));
          return true;
        }
      }
      Constraint::Convert { val, into_type } => {
        if let Some(t) = self.get_type(*val) {
          fn abstract_contains(t : Type, into_type : Type) -> bool {
            if let Type::Abstract(abs_t) = t {
              return abs_t.contains_type(into_type);
            }
            false
          }
          if
            abstract_contains(t, *into_type) ||
            (t.pointer() && into_type.pointer()) ||
            (t.number() && into_type.number()) ||
            (t.pointer() && into_type.unsigned_int()) ||
            (t.unsigned_int() && into_type.pointer())
          {
            return true;
          }
        }
      }
      Constraint::GlobalDef{ name, type_symbol, initialiser, loc } => {
        if let Some(t) = self.get_type(*type_symbol) {
          let name = self.arena.alloc_str(name);
          let g = GlobalDefinition {
            module_id: self.t.new_module_id(),
            name,
            initialiser: *initialiser,
            type_tag: t,
            loc: *loc,
          };
          self.t.create_global(self.arena.alloc(g));
          return true;
        }
      }
      Constraint::GlobalReference { node, name, result } => {
        let unknown = Type::Abstract(AbstractType::Any);
        let t = self.get_type(*result).unwrap_or(unknown);
        if let Some(r) = self.find_global(&name, t) {
          if let Ok(g) = r {
            self.register_def(*node, g.def);
            self.set_type(*result, g.concrete_type);
          }
          return true;
        }
      }
      Constraint::FieldAccess{ container, field, result } => {
        let ct = self.get_type(*container);
        if let Some(mut ct) = ct {
          // Dereference any pointers
          while let Type::Ptr(inner) = ct {
            ct = *inner;
          }
          if let Type::Def(name) = ct { 
            if let Some(def) = self.t.find_type_def(&name) {
              let f = def.fields.iter().find(|(n, _)| n.name == field.name);
              if let Some((_, t)) = f.cloned() {
                self.set_type(*result, t);
              }
              else {
                let s = format!("type '{}' has no field '{}'", def.name, field.name);
                self.errors.push(error_raw(field.loc, s));
              }
              return true;
            }
          }
          else {
            let s = format!("type {} has no fields", ct);
            self.errors.push(error_raw(field.loc, s));
            return true;
          }
        }
      }
      Constraint::Array{ array, element } => {
        if let Some(array_type) = self.get_type(*array) {
          if let Type::Array(element_type) = array_type {
            self.set_type(*element, *element_type);
            return true;
          }
        }
        if let Some(element_type) = self.get_type(*element) {
          let element_type = self.arena.alloc(element_type);
          self.set_type(*array, Type::Array(element_type));
          return true;
        }
      }
    }
    false
  }

  fn try_resolve_abstract_types(&mut self) -> bool {
    let mut count = 0;
    for r in self.resolved.values_mut() {
      if let Type::Abstract(ab) = r {
        if let Some(t) = ab.default_type() {
          *r = t;
          count += 1;
        }
      }
    }
    count > 0
  }

  fn infer(&mut self) {
    println!("To resolve: {}", self.c.symbols.len());
    let mut unused_constraints = vec![];
    for c in self.c.constraints.iter() {
      if !self.process_constraint(c) {
        unused_constraints.push(c);
      }
    }
    let mut total_passes = 1;
    while unused_constraints.len() > 0 {
      total_passes += 1;
      let remaining_before_pass = unused_constraints.len();
      unused_constraints.retain(|c| !self.process_constraint(c));
      // Continue if some constraints were resolved in the last pass
      if unused_constraints.len() < remaining_before_pass {
        continue;
      }
      // Continue if some literals can be hardened into specific types
      if self.try_resolve_abstract_types() {
        continue;
      }
      break;
    }
    println!("\nPasses taken: {}\n", total_passes);
    
    // Generate errors for unresolved constraints
    for c in unused_constraints.iter() {
      self.unresolved_constraint_error(c);
    }

    // Sanity check to make sure that programs with unresolved symbols contain errors
    let unresolved_symbol_count = self.c.symbols.len() - self.resolved.len();
    if unresolved_symbol_count > 0 && self.errors.len() == 0 {
      panic!("Symbol unresolved! Some kind of error should be generated!");
    }

    // Assign types to all of the nodes
    for (n, ts) in self.c.node_symbols.iter() {
      let t = self.get_type(*ts).unwrap();
      // Make sure the type isn't abstract
      if let Some(t) = t.to_concrete(self.arena) {
        self.cg.node_type.insert(*n, t);
      }
      else {
        let loc = self.loc(*ts);
        let e = error_raw(loc, "unresolved type");
        self.errors.push(e);
      }
    }

    // Print errors (if there are any)
    if self.errors.len() > 0 {
      println!("\nErrors:");
      for e in self.errors.iter() {
        println!("         {}", e);
      }
      println!();
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct TypeSymbol(u64);

pub enum Function {
  Value(TypeSymbol),
  Name(Symbol),
}

pub enum Constraint {
  Assert(TypeSymbol, Type),
  Equalivalent(TypeSymbol, TypeSymbol),
  Array{ array : TypeSymbol, element : TypeSymbol },
  Convert{ val : TypeSymbol, into_type : Type },
  FieldAccess {
    container : TypeSymbol,
    field : Symbol,
    result : TypeSymbol,
  },
  Constructor {
    type_name : Ap<str>,
    fields : Vec<(Option<Symbol>, TypeSymbol)>,
    result : TypeSymbol,
  },
  FunctionDef {
    name : Ap<str>,
    return_type : TypeSymbol,
    args : Vec<(Symbol, TypeSymbol)>,
    body : NodeId,
    loc : TextLocation,
  },
  FunctionCall {
    node : NodeId,
    function : Function,
    args : Vec<(Option<SymbolId>, TypeSymbol)>,
    result : TypeSymbol,

    /// this just exists to prevent repeated arena allocations when trying to resolve
    /// the function call. It's mutated a lot, which is pretty unsafe, so it shouldn't
    /// be assigned to anything.
    mut_sig : Ap<FunctionSignature>,
  },
  GlobalDef {
    name: Ap<str>,
    type_symbol: TypeSymbol,
    initialiser: GlobalInit,
    loc: TextLocation,
  },
  GlobalReference {
    node : NodeId,
    name : Ap<str>,
    result : TypeSymbol,
  },
}

struct Constraints {
  symbols : HashMap<TypeSymbol, TextLocation>,
  node_symbols : HashMap<NodeId, TypeSymbol>,
  variable_symbols : HashMap<SymbolId, TypeSymbol>,
  constraints : Vec<Constraint>,
}

impl Constraints {
  fn new() -> Self {
    Constraints {
      symbols: HashMap::new(),
      node_symbols: HashMap::new(),
      variable_symbols: HashMap::new(),
      constraints: vec![],
    }
  }

  fn loc(&self, ts : TypeSymbol) -> TextLocation {
    *self.symbols.get(&ts).unwrap()
  }
}

fn gather_constraints(
  arena : &Arena,
  t : &mut TypeDirectory,
  cg : &mut CodegenInfo,
  gen : &mut UIDGenerator,
  c : &mut Constraints,
  errors : &mut Vec<Error>,
  n : &Nodes)
{
  let mut type_def_refs = vec![];
  let mut gc = GatherConstraints::new(arena, t, cg, gen, c, errors, &mut type_def_refs);
  gc.process_node(n, n.root);
  for (name, loc) in gc.type_def_refs.iter() {
    if let Some(def) = gc.t.find_type_def(name) {
      gc.cg.type_def_references.insert(def.name, def);
    }
    else {
      let e = error_raw(loc, format!("No type definition named '{}' found.", name));
      gc.errors.push(e);
    }
  }
}

struct GatherConstraints<'l, 't> {
  arena : &'l Arena,
  labels : HashMap<LabelId, TypeSymbol>,
  t : &'l mut TypeDirectory<'t>,
  cg : &'l mut CodegenInfo,
  gen : &'l mut UIDGenerator,
  c : &'l mut Constraints,
  errors : &'l mut Vec<Error>,
  type_def_refs : &'l mut Vec<(Ap<str>, TextLocation)>,
}

impl <'l, 't> GatherConstraints<'l, 't> {

  fn new(
    arena : &'l Arena,
    t : &'l mut TypeDirectory<'t>,
    cg : &'l mut CodegenInfo,
    gen : &'l mut UIDGenerator,
    c : &'l mut Constraints,
    errors : &'l mut Vec<Error>,
    type_def_refs : &'l mut Vec<(Ap<str>, TextLocation)>,
  ) -> Self
  {
    GatherConstraints {
      labels: HashMap::new(),
      arena, t, cg, gen, c,
      errors, type_def_refs,
    }
  }

  fn log_error<V>(&mut self, r : Result<V, Error>) -> Option<V> {
    match r {
      Ok(v) => Some(v),
      Err(e) => { self.errors.push(e); None } 
    }
  }

  fn type_symbol(&mut self, loc : TextLocation) -> TypeSymbol {
    let ts = TypeSymbol(self.gen.next().into());
    self.c.symbols.insert(ts, loc);
    ts
  }

  fn node_to_symbol(&mut self, n : &Node) -> TypeSymbol {
    if let Some(ts) = self.c.node_symbols.get(&n.id) { *ts }
    else {
      let ts = self.type_symbol(n.loc);
      self.c.node_symbols.insert(n.id, ts);
      ts
    }
  }

  fn variable_to_type_symbol(&mut self, v : &RefSymbol) -> TypeSymbol {
    if let Some(ts) = self.c.variable_symbols.get(&v.id) { *ts }
    else {
      let ts = self.type_symbol(v.loc);
      self.c.variable_symbols.insert(v.id, ts);
      ts
    }
  }

  fn constraint(&mut self, c : Constraint) {
    self.c.constraints.push(c);
  }

  fn equalivalent(&mut self, a : TypeSymbol, b : TypeSymbol) {
    self.constraint(Constraint::Equalivalent(a, b));
  }

  fn assert(&mut self, ts : TypeSymbol, t : PType) {
    self.constraint(Constraint::Assert(ts, Type::Prim(t)));
  }

  fn assert_type(&mut self, ts : TypeSymbol, t : Type) {
    self.constraint(Constraint::Assert(ts, t));
  }

  fn tagged_symbol(&mut self, ts : TypeSymbol, type_expr : &Option<Box<Expr>>) {
    if let Some(type_expr) = type_expr {
      if let Some(t) = self.try_expr_to_type(type_expr) {
        self.assert_type(ts, t);
      }
    }
  }

  fn symbol(&self, s : &RefSymbol) -> Symbol {
    Symbol { id: s.id, name: self.arena.alloc_str(&s.name), loc: s.loc }
  }

  fn process_node(&mut self, n : &Nodes, id : NodeId)-> TypeSymbol {
    let node = n.node(id);
    let ts = self.node_to_symbol(node);
    match &node.content {
      Content::Literal(val) => {
        use Val::*;
        let t = match val {
          F64(_) | F32(_) => {
            Type::Abstract(AbstractType::Float)
          }
          I64(_) | I32(_) | U64(_) | U32(_) | U16(_) | U8(_) => {
            Type::Abstract(AbstractType::Integer)
          }
          Bool(_) => Type::Prim(PType::Bool),
          Void => Type::Prim(PType::Void),
          String(_) => {
            self.type_def(node.loc, self.arena.alloc_str("string"))
          }
        };
        self.assert_type(ts, t);
      }
      Content::VariableInitialise{ name, type_tag, value, var_scope } => {
        self.assert(ts, PType::Void);
        let var_type_symbol = match var_scope {
          VarScope::Local => self.variable_to_type_symbol(name),
          VarScope::Global(_) => self.type_symbol(name.loc),
        };
        self.tagged_symbol(var_type_symbol, type_tag);
        let vid = self.process_node(n, *value);
        self.equalivalent(var_type_symbol, vid);
        if let VarScope::Global(global_type) = *var_scope {
          let initialiser = match global_type {
            GlobalType::CBind => GlobalInit::CBind,
            GlobalType::Normal => GlobalInit::Expression(*value),
          };
          let name = self.arena.alloc_str(&name.name);
          self.constraint(Constraint::GlobalDef{
            name,
            type_symbol: var_type_symbol,
            initialiser,
            loc: node.loc,
          });          
        }
      }
      Content::Assignment{ assignee , value } => {
        self.assert(ts, PType::Void);
        let a = self.process_node(n, *assignee);
        let b = self.process_node(n, *value);
        self.equalivalent(a, b);
      }
      Content::IfThen{ condition, then_branch } => {
        self.assert(ts, PType::Void);
        let cond = self.process_node(n, *condition);
        let then_br = self.process_node(n, *then_branch);
        self.assert(cond, PType::Bool);
        self.assert(then_br, PType::Void);
      }
      Content::IfThenElse{ condition, then_branch, else_branch } => {
        let cond = self.process_node(n, *condition);
        let then_br = self.process_node(n, *then_branch);
        let else_br = self.process_node(n, *else_branch);
        self.equalivalent(ts, then_br);
        self.assert(cond, PType::Bool);
        self.equalivalent(then_br, else_br);
      }
      Content::Block(ns) => {
        let len = ns.len();
        if len > 0 {
          for child in &ns[0..(len-1)] {
            self.process_node(n, *child);
          }
          let c = self.process_node(n, ns[len-1]);
          self.equalivalent(ts, c);
        }
        else {
          self.assert(ts, PType::Void);
        }
      }
      Content::Quote(_e) => {
        let t = self.arena.alloc(self.type_def(node.loc, self.arena.alloc_str("expr")));
        self.assert_type(ts, Type::Ptr(t));
      }
      Content::Reference{ name, refers_to } => {
        if let Some(refers_to) = refers_to {
          let var_type = self.variable_to_type_symbol(n.symbol(*refers_to));
          self.equalivalent(ts, var_type);
        }
        else {
          let name = self.arena.alloc_str(&name);
          self.constraint(Constraint::GlobalReference{ node: id, name, result: ts });
        }
      }
      Content::FunctionDefinition{ name, args, return_tag, body } => {
        self.assert(ts, PType::Void);
        let mut ts_args : Vec<(Symbol, TypeSymbol)> = vec![];
        for (arg, type_tag) in args.iter() {
          let arg_type_symbol = self.variable_to_type_symbol(arg);
          self.tagged_symbol(arg_type_symbol, type_tag);
          ts_args.push((self.symbol(arg), arg_type_symbol));
        }
        let body_ts = {
          // Need new scope stack for new function
          let mut gc =
            GatherConstraints::new(self.arena, self.t, self.cg, self.gen, self.c, self.errors, self.type_def_refs);
          gc.process_node(n, *body)
        };
        self.tagged_symbol(body_ts, return_tag);
        let name = self.arena.alloc_str(&name);
        let f = Constraint::FunctionDef {
          name, args: ts_args,
          return_type: body_ts, body: *body, loc: node.loc };
        self.constraint(f);
      }
      Content::CBind { name, type_tag } => {
        self.assert(ts, PType::Void);
        let cbind_ts = self.type_symbol(node.loc);
        if let Some(t) = self.try_expr_to_type(type_tag) {
          self.assert_type(cbind_ts, t);
          let name = self.arena.alloc_str(&name);
          let g = GlobalDefinition {
            module_id: self.t.new_module_id(),
            name,
            initialiser: GlobalInit::CBind,
            type_tag: t,
            loc: node.loc,
          };
          self.t.create_global(self.arena.alloc(g));
        }
      }
      Content::TypeDefinition{ name, kind, fields } => {
        self.assert(ts, PType::Void);
        if self.t.find_type_def(name.as_ref()).is_some() {
          let e = error_raw(node.loc, "type with this name already defined");
          self.errors.push(e)
        }
        else {
          // TODO: check for duplicate fields?
          let mut typed_fields = vec![];
          for (field, type_tag) in fields.iter() {
            if let Some(t) = self.try_expr_to_type(type_tag.as_ref().unwrap()) {
              typed_fields.push((self.symbol(field), t));
            }
          }
          // TODO: Generics?
          let name = self.arena.alloc_str(name);
          let def = TypeDefinition {
            name,
            fields: self.arena.alloc_slice(&typed_fields),
            kind: *kind,
            drop_function: None, clone_function: None,
            definition_location: node.loc,
          };
          self.t.create_type_def(self.arena.alloc(def));
        }
      }
      Content::TypeConstructor{ name, field_values } => {
        let mut fields = vec![];
        for (field, value) in field_values.iter() {
          let field_type_symbol = self.process_node(n, *value);
          let field = field.as_ref().map(|f| self.symbol(f));
          fields.push((field, field_type_symbol));
        }
        let type_name = self.arena.alloc_str(&name);
        let tc = Constraint::Constructor{ type_name, fields, result: ts };
        let def_type = self.type_def(node.loc, type_name);
        self.assert_type(ts, def_type);
        self.constraint(tc);
      }
      Content::FieldAccess{ container, field } => {
        let fa = Constraint::FieldAccess {
          container: self.process_node(n, *container),
          field: self.symbol(field),
          result: ts,
        };
        self.constraint(fa);
      }
      Content::ArrayLiteral(ns) => {
        let element_ts = self.type_symbol(node.loc);
        for element in ns.iter() {
          let el = self.process_node(n, *element);
          self.equalivalent(el, element_ts);
        }
        self.constraint(Constraint::Array{ array: ts, element: element_ts });
      }
      Content::FunctionCall{ function, args } => {
        let function = match function {
          FunctionNode::Name(name) => Function::Name(self.symbol(name)),
          FunctionNode::Value(val) => {
            let val = self.process_node(n, *val);
            Function::Value(val)
          }
        };
        let unknown = Type::Abstract(AbstractType::Any);
        let mut_sig = FunctionSignature {
          args: self.arena.slice_of(args.len(), unknown).into_ap(),
          return_type: unknown,
        };
        let fc = Constraint::FunctionCall {
          node: id,
          function,
          args: args.iter().map(|id| (None, self.process_node(n, *id))).collect(),
          result: ts,
          mut_sig: self.arena.alloc(mut_sig),
        };
        self.constraint(fc);
      }
      Content::While{ condition, body } => {
        self.assert(ts, PType::Void);
        let cond = self.process_node(n, *condition);
        let body = self.process_node(n, *body);
        self.assert(cond, PType::Bool);
        self.assert(body, PType::Void);
      }
      Content::Convert{ from_value, into_type } => {
        let v = self.process_node(n, *from_value);
        if let Some(t) = self.try_expr_to_type(into_type) {
          self.assert_type(ts, t);
          let c = Constraint::Convert { val: v, into_type: t };
          self.constraint(c);
        }
      }
      Content::SizeOf{ type_tag } => {
        if let Some(tid) = self.try_expr_to_type(type_tag) {
          self.cg.sizeof_info.insert(node.id, tid);
        }
        self.assert(ts, PType::U64);
      }
      Content::Label{ label, body } => {
        self.labels.insert(*label, ts);
        let body = self.process_node(n, *body);
        self.equalivalent(ts, body);
      }
      Content::BreakToLabel{ label, return_value } => {
        self.assert(ts, PType::Void);
        let label_ts = *self.labels.get(label).unwrap();
        if let Some(v) = return_value {
          let v = self.process_node(n, *v);
          self.equalivalent(label_ts, v);
        }
        else {
          self.assert(label_ts, PType::Void);
        }
      }
    }
    ts
  }

  fn try_expr_to_type(&mut self, e : &Expr) -> Option<Type> {
    let r = self.expr_to_type(e);
    self.log_error(r)
  }

  fn type_def(&mut self, loc : TextLocation, name : Ap<str>) -> Type {
    self.type_def_refs.push((name, loc));
    Type::Def(name)
  }

  /// Converts expression into type. Logs symbol error if definition references a type that hasn't been defined yet
  /// These symbol errors may be resolved later, when the rest of the module has been checked.
  fn expr_to_type(&mut self, expr : &Expr) -> Result<Type, Error> {
    if let Some(name) = expr.try_symbol() {
      if let Some(t) = Type::from_string(name) {
        return Ok(t);
      }
      let name = self.arena.alloc_str(name);
      return Ok(self.type_def(expr.loc, name));
    }
    match expr.try_construct() {
      Some(("fun", es)) => {
        if let Some(args) = es.get(0) {
          let args =
            args.children().iter()
            .map(|e| {
              let e = if let Some((":", [_name, tag])) = e.try_construct() {tag} else {e};
              self.expr_to_type(e)
            })
            .collect::<Result<Vec<Type>, Error>>()?;
          let return_type = if let Some(t) = es.get(1) {
            self.expr_to_type(t)?
          }
          else {
            PType::Void.into()
          };
          let args = self.arena.alloc_slice(args.as_slice());
          let sig = self.arena.alloc(FunctionSignature{ args, return_type});
          return Ok(Type::Fun(sig));
        }
      }
      Some(("call", [name, t])) => {
        match name.unwrap_symbol()? {
          "ptr" => {
            let t = self.arena.alloc(self.expr_to_type(t)?);
            return Ok(Type::Ptr(t))
          }
          "array" => {
            let t = self.arena.alloc(self.expr_to_type(t)?);
            return Ok(Type::Array(t))
          }
          _ => (),
        }
      }
      _ => ()
    }
    error(expr, "invalid type expression")
  }
}
