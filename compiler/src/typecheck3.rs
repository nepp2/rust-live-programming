
use std::rc::Rc;
use std::fmt::Write;

use crate::error::{Error, error, error_raw, TextLocation};
use crate::expr::{StringCache, RefStr, Expr, ExprTag};

use std::collections::HashMap;
use itertools::Itertools;

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
  Void,
  F64,
  F32,
  I64,
  U64,
  I32,
  U32,
  U16,
  U8,
  Bool,
  Fun(Rc<FunctionSignature>),
  Def(RefStr),
  Ptr(Box<Type>),
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

impl Type {
  pub fn from_string(s : &str) -> Option<Type> {
    match s {
      "f64" => Some(Type::F64),
      "f32" => Some(Type::F32),
      "bool" => Some(Type::Bool),
      "i64" => Some(Type::I64),
      "u64" => Some(Type::U64),
      "i32" => Some(Type::I32),
      "u32" => Some(Type::U32),
      "u16" => Some(Type::U16),
      "u8" => Some(Type::U8),
      "()" => Some(Type::Void),
      "" => Some(Type::I64),
      _ => None,
    }
  }

  pub fn float(&self) -> bool {
    match self { Type::F32 | Type::F64 => true, _ => false }
  }

  pub fn unsigned_int(&self) -> bool {
    match self { Type::U64 | Type::U32 | Type::U16 | Type::U8 => true, _ => false }
  }

  pub fn signed_int(&self) -> bool {
    match self { Type::I64 | Type::I32 => true, _ => false }
  }

  pub fn int(&self) -> bool {
    self.signed_int() || self.unsigned_int()
  }

  pub fn pointer(&self) -> bool {
    match self { Type::Ptr(_) | Type::Fun(_) => true, _ => false }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
  Struct, Union
}

#[derive(Clone, Debug)]
pub struct TypeDefinition {
  pub name : RefStr,
  pub fields : Vec<(RefStr, Type)>,
  pub kind : TypeKind,
}

#[derive(Debug)]
pub enum FunctionImplementation {
  Normal(TypedNode),
  CFunction(Option<usize>),
  Intrinsic,
}

#[derive(Debug)]
pub struct FunctionDefinition {
  pub name : RefStr,
  pub args : Vec<RefStr>,
  pub signature : Rc<FunctionSignature>,
  pub implementation : FunctionImplementation,
}

#[derive(Debug, PartialEq)]
pub struct FunctionSignature {
  pub return_type : Type,
  pub args : Vec<Type>,
}

impl PartialEq for TypeDefinition {
  fn eq(&self, rhs : &Self) -> bool {
    self.name == rhs.name
  }
}

#[derive(Debug)]
pub enum Content {
  Literal(Val),
  SymbolReference(RefStr),
  GlobalDefinition(RefStr, Box<TypedNode>),
  GlobalReference(RefStr),
  VariableDefinition(RefStr, Box<TypedNode>),
  VariableReference(RefStr),
  Assignment(Box<(TypedNode, TypedNode)>),
  IfThen(Box<(TypedNode, TypedNode)>),
  IfThenElse(Box<(TypedNode, TypedNode, TypedNode)>),
  Block(Vec<TypedNode>),
  Quote(Box<Expr>),
  FunctionReference(RefStr),
  FunctionDefinition(RefStr),
  CFunctionPrototype(RefStr),
  TypeDefinition(RefStr),
  StructInstantiate(RefStr, Vec<TypedNode>),
  UnionInstantiate(RefStr, Box<(RefStr, TypedNode)>),
  FieldAccess(Box<(TypedNode, RefStr)>, usize),
  Index(Box<(TypedNode, TypedNode)>),
  ArrayLiteral(Vec<TypedNode>),
  FunctionCall(Box<TypedNode>, Vec<TypedNode>),
  IntrinsicCall(RefStr, Vec<TypedNode>),
  While(Box<(TypedNode, TypedNode)>),
  ExplicitReturn(Option<Box<TypedNode>>),
  Convert(Box<TypedNode>),
  Deref(Box<TypedNode>),
  SizeOf(Box<Type>),
  Break,
}

#[derive(Debug)]
pub struct TypedNode {
  pub type_tag : Type,
  pub content : Content,
  pub loc : TextLocation,
}

impl TypedNode {
  fn assert_type(&self, expected : Type) -> Result<(), Error> {
    if self.type_tag == expected {
      Ok(())
    }
    else {
      error(self.loc, format!("expected type {:?}, found type {:?}", expected, self.type_tag))
    }
  }
}

fn node(expr : &Expr, type_tag : Type, content : Content) -> TypedNode {
  TypedNode {
    type_tag,
    content,
    loc: expr.loc,
  }
}

pub struct TypedModule {
  pub types : HashMap<RefStr, TypeDefinition>,
  pub functions : HashMap<RefStr, FunctionDefinition>,
  pub globals : HashMap<RefStr, Type>,
}

impl TypedModule {
  fn new() -> TypedModule {
    TypedModule{ types: HashMap::new(), functions: HashMap::new(), globals: HashMap::new() }
  }
}

/*
  Namespacing examples:
    - module + function name
    - module + type
    - module + function + type?
    - varname + function + scope
*/

pub struct TypeChecker<'l> {
  new_module : &'l TypedModule,
  modules : &'l [TypedModule],
  local_symbol_table : &'l HashMap<RefStr, usize>,

  cache: &'l StringCache,
}

pub struct FunctionChecker<'l> {
  is_top_level : bool,
  typecheck : &'l TypeChecker<'l>,
  variables: HashMap<RefStr, Type>,
  new_symbols : TypedModule,

  /// Tracks which variables are available, when.
  /// Used to rename variables with clashing names.
  scope_map: Vec<HashMap<RefStr, RefStr>>,

  cache: &'l StringCache,
}

impl <'l> FunctionChecker<'l> {

  fn symbol_defined(&self, name : &str) -> bool {
    self.find_global(name).is_some()
      || self.find_function(name).is_some()
      || self.find_type_def(name).is_some()
  }

  fn find_global(&self, name : &str) -> Option<&Type> {
    panic!()
  }

  fn find_function(&self, name : &str) -> Option<&FunctionDefinition> {
    panic!()
  }

  fn find_type_def(&self, name : &str) -> Option<&TypeDefinition> {
    panic!()
  }

  fn to_type(&self, expr : &Expr) -> Result<Type, Error> {
    self.typecheck.to_type(expr)
  }

  fn get_scoped_variable_name(&self, name : &RefStr) -> RefStr {
    for m in self.scope_map.iter().rev() {
      if let Some(n) = m.get(name) {
        return n.clone();
      }
    }
    return name.clone();
  }

  fn create_scoped_variable_name(&mut self, name : RefStr) -> RefStr {
    let mut unique_name = name.to_string();
    let mut i = 0;
    while self.find_global(unique_name.as_str()).is_some() ||
      self.variables.contains_key(unique_name.as_str())
    {
      unique_name.clear();
      i += 1;
      write!(&mut unique_name, "{}#{}", name, i).unwrap();
    }
    let unique_name : RefStr = unique_name.into();
    self.scope_map.last_mut().unwrap().insert(name, unique_name.clone());
    unique_name.clone()
  }

  fn match_intrinsic(name : &str, args : &[TypedNode]) -> Option<Type> {
    match args {
      [a, b] => match (&a.type_tag, &b.type_tag) {
        (Type::F64, Type::F64) => match name {
          "+" | "-" | "*" | "/" => Some(Type::F64),
          ">" | ">="| "<" | "<=" | "==" => Some(Type::Bool),
          _ => None,
        }
        (Type::I64, Type::I64) => match name {
          "+" | "-" | "*" | "/" => Some(Type::I64),
          ">" | ">="| "<" | "<=" | "==" => Some(Type::Bool),
          _ => None,
        }
        _ => None
      }
      [a] => match (&a.type_tag, name) {
        (Type::F64, "unary_-") => Some(Type::F64),
        (Type::I64, "unary_-") => Some(Type::I64),
        (Type::Bool, "unary_!") => Some(Type::Bool),
        (t, "unary_ref") => Some(Type::Ptr(Box::new(t.clone()))),
        _ => None,
      }
      _ => None,
    }
  }

  fn tree_to_ast(&mut self, expr : &Expr) -> Result<TypedNode, Error> {
    let instr = expr.symbol_unwrap()?;
    let children = expr.children.as_slice();
    match (instr, children) {
      ("call", exprs) => {
        let args =
              exprs[1..].iter().map(|e| self.to_ast(e))
              .collect::<Result<Vec<TypedNode>, Error>>()?;
        if let Some(function_name) = exprs[0].symbol_unwrap().ok() {
          let op_tag = FunctionChecker::match_intrinsic(
            function_name, args.as_slice());
          if let Some(op_tag) = op_tag {
            return Ok(node(expr, op_tag, Content::IntrinsicCall(self.cache.get(function_name), args)))
          }
        }
        let function_value = self.to_ast(&exprs[0])?;
        if let Type::Fun(sig) = &function_value.type_tag {
          if sig.args.len() != args.len() {
            return error(expr, "incorrect number of arguments passed");
          }
          let return_type = sig.return_type.clone();
          let content = Content::FunctionCall(Box::new(function_value), args);
          return Ok(node(expr, return_type, content));
        }
        error(&exprs[0], "value is not a function")
      }
      ("sizeof", [t]) => {
        let type_tag = self.to_type(t)?;
        return Ok(node(expr, Type::U64, Content::SizeOf(Box::new(type_tag))));
      }
      ("as", [a, b]) => {
        let a = self.to_ast(a)?;
        let t = self.to_type(b)?;
        Ok(node(expr, t, Content::Convert(Box::new(a))))
      }
      ("&&", [a, b]) => {
        let a = self.to_ast(a)?;
        let b = self.to_ast(b)?;
        Ok(node(expr, Type::Bool, Content::IntrinsicCall(self.cache.get(instr), vec!(a, b))))
      }
      ("||", [a, b]) => {
        let a = self.to_ast(a)?;
        let b = self.to_ast(b)?;
        Ok(node(expr, Type::Bool, Content::IntrinsicCall(self.cache.get(instr), vec!(a, b))))
      }
      ("let", exprs) => {
        let name_expr = &exprs[0];
        let name = self.cache.get(name_expr.symbol_unwrap()?);
        let v = Box::new(self.to_ast(&exprs[1])?);
        // The first scope is used for function arguments. The second
        // is the top level of the function.
        let c = if self.is_top_level && self.scope_map.len() == 2 {
          // global variable
          if self.symbol_defined(&name) {
            return error(name_expr.loc, "symbol with this name already defined");
          }
          self.new_module.globals.insert(name.clone(), v.type_tag.clone());
          Content::GlobalDefinition(name, v)
        }
        else {
          // local variable
          let scoped_name = self.create_scoped_variable_name(name);
          self.variables.insert(scoped_name.clone(), v.type_tag.clone());
          Content::VariableDefinition(scoped_name, v)
        };
        Ok(node(expr, Type::Void, c))
      }
      // TODO this is a very stupid approach
      ("quote", [e]) => {
        Ok(node(expr, Type::Ptr(Box::new(Type::U8)), Content::Quote(Box::new(e.clone()))))
      }
      ("=", [assign_expr, value_expr]) => {
        let a = self.to_ast(assign_expr)?;
        let b = self.to_ast(value_expr)?;
        Ok(node(expr, Type::Void, Content::Assignment(Box::new((a, b)))))
      }
      ("return", exprs) => {
        if exprs.len() > 1 {
          return error(expr, format!("malformed return expression"));
        }
        let (return_val, type_tag) =
          if exprs.len() == 1 {
            let v = self.to_ast(&exprs[0])?;
            let t = v.type_tag.clone();
            (Some(Box::new(v)), t)
          }
          else {
            (None, Type::Void)
          };
        Ok(node(expr, type_tag, Content::ExplicitReturn(return_val)))
      }
      ("while", [condition_node, body_node]) => {
        let condition = self.to_ast(condition_node)?;
        let body = self.to_ast(body_node)?;
        Ok(node(expr, Type::Void, Content::While(Box::new((condition, body)))))
      }
      ("if", exprs) => {
        if exprs.len() > 3 {
          return error(expr, "malformed if expression");
        }
        let condition = self.to_ast(&exprs[0])?;
        condition.assert_type(Type::Bool)?;
        let then_branch = self.to_ast(&exprs[1])?;
        if exprs.len() == 3 {
          let else_branch = self.to_ast(&exprs[2])?;
          if then_branch.type_tag != else_branch.type_tag {
            return error(expr, "if/else branch type mismatch");
          }
          let t = then_branch.type_tag.clone();
          let c = Content::IfThenElse(Box::new((condition, then_branch, else_branch)));
          Ok(node(expr, t, c))
        }
        else {
          Ok(node(expr, Type::Void, Content::IfThen(Box::new((condition, then_branch)))))
        }
      }
      ("block", exprs) => {
        self.scope_map.push(HashMap::new());
        let nodes = exprs.iter().map(|e| self.to_ast(e)).collect::<Result<Vec<TypedNode>, Error>>()?;
        self.scope_map.pop();
        let tag = nodes.last().map(|n| n.type_tag.clone()).unwrap_or(Type::Void);
        Ok(node(expr, tag, Content::Block(nodes)))
      }
      ("cfun", exprs) => {
        let name_expr = &exprs[0];
        let name = self.cache.get(name_expr.symbol_unwrap()?);
        if self.symbol_defined(&name) {
          return error(name_expr.loc, "symbol with this name already defined");
        }
        let args_exprs = exprs[1].children.as_slice();
        let return_type_expr = &exprs[2];
        let mut arg_names = vec!();
        let mut arg_types = vec!();
        for (name_expr, type_expr) in args_exprs.iter().tuples() {
          let name = self.cache.get(name_expr.symbol_unwrap()?);
          let type_tag = self.to_type(type_expr)?;
          if type_tag == Type::Void {
            return error(expr, "functions args cannot be void");
          }
          arg_names.push(name);
          arg_types.push(type_tag);
        }
        let return_type = self.to_type(return_type_expr)?;
        let signature = Rc::new(FunctionSignature {
          return_type,
          args: arg_types,
        });
        let address = self.typecheck.local_symbol_table.get(&name).map(|v| *v);
        if address.is_none() {
          // TODO: check the signature of the function too
          println!("Warning: C function '{}' not linked. LLVM linker may link it instead.", name);
          // return error(expr, "tried to bind non-existing C function")
        }
        let def = FunctionDefinition {
          name: name.clone(),
          args: arg_names,
          signature,
          implementation: FunctionImplementation::CFunction(address),
        };
        self.new_module.functions.insert(name.clone(), def);
        
        Ok(node(expr, Type::Void, Content::CFunctionPrototype(name)))
      }
      ("fun", exprs) => {
        let name_expr = &exprs[0];
        let name = self.cache.get(name_expr.symbol_unwrap()?);
        if self.symbol_defined(&name) {
          return error(name_expr.loc, "symbol with this name already defined");
        }
        let args_exprs = exprs[1].children.as_slice();
        let function_body = &exprs[2];
        let mut arg_names = vec!();
        let mut arg_types = vec!();
        for (name_expr, type_expr) in args_exprs.iter().tuples() {
          let name = self.cache.get(name_expr.symbol_unwrap()?);
          let type_tag = self.to_type(type_expr)?;
          if type_tag == Type::Void {
            return error(expr, "functions args cannot be void");
          }
          arg_names.push(name);
          arg_types.push(type_tag);
        }
        let args = arg_names.iter().cloned().zip(arg_types.iter().cloned()).collect();
        let mut type_checker =
          TypeChecker::new(
            false, self.new_module, self.modules, args,
            self.local_symbol_table, self.cache);
        let body = type_checker.to_ast(function_body)?;
        let signature = Rc::new(FunctionSignature {
          return_type: body.type_tag.clone(),
          args: arg_types,
        });
        let def = FunctionDefinition {
          name: name.clone(),
          args: arg_names,
          signature,
          implementation: FunctionImplementation::Normal(body),
        };
        self.new_module.functions.insert(name.clone(), def);
        Ok(node(expr, Type::Void, Content::FunctionDefinition(name)))
      }
      ("union", exprs) => {
        let name = exprs[0].symbol_unwrap()?;
        Ok(node(expr, Type::Void, Content::TypeDefinition(self.cache.get(name))))
      }
      ("struct", exprs) => {
        let name = exprs[0].symbol_unwrap()?;
        Ok(node(expr, Type::Void, Content::TypeDefinition(self.cache.get(name))))
      }
      ("type_instantiate", exprs) => {
        if exprs.len() < 1 || exprs.len() % 2 == 0 {
          return error(expr, format!("malformed type instantiation"));
        }
        let name_expr = &exprs[0];
        let field_exprs = &exprs[1..];
        let name = name_expr.symbol_unwrap()?;
        let fields =
          field_exprs.iter().tuples().map(|(name, value)| {
            let value = self.to_ast(value)?;
            Ok((name, value))
          })
          .collect::<Result<Vec<(&Expr, TypedNode)>, Error>>()?;
        let def =
          self.find_type_def(name)
          .ok_or_else(|| error_raw(name_expr, "no type with this name exists"))?;
        match &def.kind {
          TypeKind::Struct => {
            if fields.len() != def.fields.len() {
              return error(expr, "wrong number of fields");
            }
            let field_iter = fields.iter().zip(def.fields.iter());
            for ((field, value), (expected_name, expected_type)) in field_iter {
              let name = field.symbol_unwrap()?;
              if name != "" && name != expected_name.as_ref() {
                return error(*field, "incorrect field name");
              }
              if &value.type_tag != expected_type {
                return error(value.loc, format!("type mismatch. expected {:?}, found {:?}", expected_type, value.type_tag));
              }
            }
            let c = Content::StructInstantiate(self.cache.get(name), fields.into_iter().map(|v| v.1).collect());
            Ok(node(expr, Type::Def(def.name.clone()), c))
          }
          TypeKind::Union => {
            if fields.len() != 1 {
              return error(expr, "must instantiate exactly one field");
            }
            let (field, value) = fields.into_iter().nth(0).unwrap();
            let name = self.cache.get(field.symbol_unwrap()?);
            if def.fields.iter().find(|(n, _)| n == &name).is_none() {
              return error(field, "field does not exist in this union");
            }
            let c = Content::UnionInstantiate(self.cache.get(name), Box::new((name, value)));
            Ok(node(expr, Type::Def(def.name.clone()), c))
          }
        }
      }
      (".", [container_expr, field_expr]) => {
        let container_val = self.to_ast(container_expr)?;
        let field_name = self.cache.get(field_expr.symbol_unwrap()?);
        let def = match &container_val.type_tag {
          Type::Def(def) => self.find_type_def(def).unwrap(),
          _ => return error(container_expr, format!("expected struct or union, found {:?}", container_val.type_tag)),
        };
        let (field_index, (_, field_type)) =
          def.fields.iter().enumerate().find(|(_, (n, _))| n==&field_name)
          .ok_or_else(|| error_raw(field_expr, "type does not have field with this name"))?;
        let field_type = field_type.clone();
        let c = Content::FieldAccess(Box::new((container_val, field_name)), field_index);
        Ok(node(expr, field_type, c))
      }
      ("literal_array", exprs) => {
        let mut elements = vec!();
        for e in exprs {
          elements.push(self.to_ast(e)?);
        }
        let t =
          if let Some(a) = elements.first() {
            for b in &elements[1..] {
              if a.type_tag != b.type_tag {
                return error(expr, format!("array initialiser contains more than one type."));
              }
            }
            a.type_tag.clone()
          }
          else {
            Type::Void
          };
        Ok(node(expr, Type::Ptr(Box::new(t)), Content::ArrayLiteral(elements)))
      }
      ("index", [array_expr, index_expr]) => {
        let array = self.to_ast(array_expr)?;
        let inner_type = match &array.type_tag {
          Type::Ptr(t) => *(t).clone(),
          _ => return error(array_expr, "expected ptr"),
        };
        let index = self.to_ast(index_expr)?;
        if index.type_tag != Type::I64 {
          return error(array_expr, "expected integer");
        }
        Ok(node(expr, inner_type, Content::Index(Box::new((array, index)))))
      }
      _ => return error(expr, "unsupported expression"),
    }
  }

  fn to_ast(&mut self, expr : &Expr) -> Result<TypedNode, Error> {
    match &expr.tag {
      ExprTag::Symbol(s) => {
        // Is this a tree?
        let children = expr.children.as_slice();
        if children.len() > 0 {
          return self.tree_to_ast(expr);
        }
        // this is just a normal symbol
        let s = self.cache.get(s.as_str());
        if s.as_ref() == "break" {
          return Ok(node(expr, Type::Void, Content::Break));
        }
        let name = self.get_scoped_variable_name(&s);
        if let Some(t) = self.variables.get(name.as_ref()) {
          return Ok(node(expr, t.clone(), Content::VariableReference(name)));
        }
        if let Some(t) = self.find_global(name.as_ref()) {
          return Ok(node(expr, t.clone(), Content::GlobalReference(name)));
        }
        if let Some(def) = self.find_function(&s) {
          return Ok(node(expr, Type::Fun(def.signature.clone()), Content::FunctionReference(s)));
        }
        error(expr, format!("unknown variable name '{}'", s))
      }
      ExprTag::LiteralString(s) => {
        let v = Val::String(s.as_str().to_string());
        let s = self.find_type_def("string").unwrap();
        Ok(node(expr, Type::Def(s.name.clone()), Content::Literal(v)))
      }
      ExprTag::LiteralFloat(f) => {
        let v = Val::F64(*f as f64);
        Ok(node(expr, Type::F64, Content::Literal(v)))
      }
      ExprTag::LiteralInt(v) => {
        let v = Val::I64(*v as i64);
        Ok(node(expr, Type::I64, Content::Literal(v)))
      }
      ExprTag::LiteralBool(b) => {
        let v = Val::Bool(*b);
        Ok(node(expr, Type::Bool, Content::Literal(v)))
      },
      ExprTag::LiteralUnit => {
        Ok(node(expr, Type::Void, Content::Literal(Val::Void)))
      },
      // _ => error(expr, "unsupported expression"),
    }
  }
}

impl <'l> TypeChecker<'l> {

  pub fn new(
    new_module : &'l TypedModule,
    modules : &'l [TypedModule],
    local_symbol_table : &'l HashMap<RefStr, usize>,
    cache : &'l mut StringCache)
      -> TypeChecker<'l>
  {
    TypeChecker {
      new_module,
      modules,
      local_symbol_table,
      cache,
    }
  }

  fn function_checker(&'l self, is_top_level : bool, variables : HashMap<RefStr, Type>) -> FunctionChecker<'l> {
    FunctionChecker::<'l> {
      is_top_level,
      typecheck: self,
      variables,
      new_symbols: TypedModule::new(),
      scope_map: vec!(),
      cache: self.cache,
    }
  }

  fn typecheck_function(&mut self, expr : &Expr) -> Result<(FunctionDefinition, TypedModule), Error> {
    if let ExprTag::Symbol(s) = &expr.tag {
      let children = expr.children.as_slice();
      match (s.as_str(), children) {
        ("fun", exprs) => {
          let name = self.cache.get(exprs[0].symbol_unwrap()?);
          if self.symbol_defined(&name) {
            return error(name_expr.loc, "symbol with this name already defined");
          }
          let args_exprs = exprs[1].children.as_slice();
          let function_body = &exprs[2];
          let mut arg_names = vec!();
          let mut arg_types = vec!();
          for (name_expr, type_expr) in args_exprs.iter().tuples() {
            let name = self.cache.get(name_expr.symbol_unwrap()?);
            let type_tag = self.to_type(type_expr)?;
            arg_names.push(name);
            arg_types.push(type_tag);
          }
          return self.typecheck_function_body(name, arg_names, arg_types, function_body, false);
        }
        ("block", exprs) => {
          // this is a top-level function
          let name = self.cache.get("top_level");
          return self.typecheck_function_body(name, vec!(), vec!(), expr, true);
        }
        _ => (),
      }
    }
    return error(expr, "unsupported expression");
  }

  fn typecheck_function_body(
    &mut self, name : RefStr,
    arg_names : Vec<RefStr>, arg_types : Vec<Type>,
    function_body : &Expr, is_top_level : bool)
      -> Result<(FunctionDefinition, TypedModule), Error>
  {
    let args = arg_names.iter().cloned().zip(arg_types.iter().cloned()).collect();
    let mut function_checker = self.function_checker(is_top_level, args);
    let body = function_checker.to_ast(function_body)?;
    let signature = Rc::new(FunctionSignature {
      return_type: body.type_tag.clone(),
      args: arg_types,
    });
    let def = FunctionDefinition {
      name: name.clone(),
      args: arg_names,
      signature,
      implementation: FunctionImplementation::Normal(body),
    };
    return Ok((def, function_checker.new_symbols));
  }

  fn find_type_def(&self, name : &str) -> Option<&TypeDefinition> {
    panic!()
  }

  /// Converts expression into type. Returns error if type references a type definition that doesn't exist.
  fn to_type(&mut self, expr : &Expr) -> Result<Type, Error> {
    let name = expr.symbol_unwrap()?;
    let params = expr.children.as_slice();
    if let Some(t) = Type::from_string(name) {
      if params.len() > 0 {
        return error(expr, "unexpected type parameters");
      }
      return Ok(t);
    }
    if name == "fun" {
      let args =
        params[0].children.as_slice().iter().map(|e| self.to_type(e))
        .collect::<Result<Vec<Type>, Error>>()?;
      let return_type = self.to_type(&params[1])?;
      return Ok(Type::Fun(Rc::new(FunctionSignature{ args, return_type})));
    }
    match (name, params) {
      ("ptr", [t]) => {
        let t = self.to_type(t)?;
        Ok(Type::Ptr(Box::new(t)))
      }
      (name, params) => {
        if params.len() > 0 {
          return error(expr, "unexpected type parameters");
        }
        if self.find_type_def(name).is_none() {
          return error(expr, format!("type '{}' does not exist", name));
        }
        return Ok(Type::Def(self.cache.get(name)));
      }
    }
  }

  fn to_type_definition(&mut self, expr : &Expr) -> Result<TypeDefinition, Error> {
    let kind = match expr.symbol_unwrap()? {
      "struct" => TypeKind::Struct,
      "union" => TypeKind::Union,
    };
    let children = expr.children.as_slice();
    if children.len() < 1 {
      return error(expr, "malformed type definition");
    }
    let name_expr = &children[0];
    let name = name_expr.symbol_unwrap()?;
    if self.find_type_def(name).is_some() {
      return error(expr, "struct with this name already defined");
    }
    // TODO: check for duplicates?
    let field_exprs = &children[1..];
    let mut fields = vec![];
    // TODO: record the field types, and check them!
    for (field_name_expr, type_expr) in field_exprs.iter().tuples() {
      let field_name = field_name_expr.symbol_unwrap()?.clone();
      let type_tag = self.to_type(type_expr)?;
      fields.push((self.cache.get(field_name), type_tag));
    }
    Ok(TypeDefinition { name: self.cache.get(name), fields, kind })
  }

  pub fn typecheck_module(&self, expr : &Expr) -> Result<TypedModule, Error> {
    let mut type_exprs = vec!();
    let mut function_exprs = vec!(expr);
    find_symbols(expr, &mut type_exprs, &mut function_exprs);

    let mut module = TypedModule { types: HashMap::new(), functions: HashMap::new(), globals: HashMap::new() };

    // check type definitions
    for e in type_exprs.into_iter() {
      let def = self.to_type_definition(e)?;
      module.types.insert(def.name.clone(), def);
    }
    let mut fns = vec!();
    loop {
      let mut errors = vec!();
      fns.append(&mut function_exprs);
      let mut initial_functions_count = fns.len();
      for function_expr in fns.drain(0..) {
        let r = self.typecheck_function(function_expr);
        match r {
          Ok((def, new_symbols)) => {
            module.functions.insert(def.name.clone(), def);
            module.functions.extend(new_symbols.functions);
            module.types.extend(new_symbols.types);
            module.globals.extend(new_symbols.globals);
          }
          Err(e) => {
            function_exprs.push(function_expr);
            errors.push(e);
          }
        }
      }
      if function_exprs.is_empty() {
        break;
      }
      if function_exprs.len() == initial_functions_count {
        return Err(errors[0]);
      }
    }

    // Try to compile the top-level, because it has the globals


    // let mut new_types = HashMap::new();
    // let types = type_exprs.iter().map(|e| self.to_type_definition(e, &mut new_types)).collect::<Result<Vec<TypeDefinition>, Error>>()?;
    // for t in types.iter() {
    //   new_types.remove(&t.name);
    // }
    // let errors = new_types.iter().collect::<Vec<_>>();
    // errors.sort_by_key(|(_, loc)| loc.start.line);
    // if let Some((name, loc)) = errors.first() {
    //   return error(*loc, format!("type '{}' does not exist", name));
    // }
    // let top_level_function = self.typecheck_top_level_function(expr)?;
    // let mut functions = vec!();
    // for e in function_exprs.iter() {
    //   let f = self.typecheck_function(e)?;
    //   functions.push(f);
    // }

    // let globals = HashMap::new(); // TODO BROKEN
    // let types = types.into_iter().map(|def| (def.name.clone(), def)).collect();
    // let functions = functions.into_iter().map(|f| (f.def.name.clone(), f)).collect();

    // Ok(TypedModule { types, functions, globals })
    panic!()
  }

}

fn find_symbols<'e>(expr : &'e Expr, types : &mut Vec<&'e Expr>, functions : &mut Vec<&'e Expr>) {
  let children = expr.children.as_slice();
  if children.len() == 0 { return }
  if let ExprTag::Symbol(s) = &expr.tag {
    match s.as_str() {
      "union" => {
      types.push(expr);
      return;
      }
      "struct" => {
      types.push(expr);
      return;
      }
      "fun" => {
      functions.push(expr);
      }
      _ => (),
    }
  }
  for c in children {
    find_symbols(c, types, functions);
  }
}