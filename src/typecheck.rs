
use error::{Error, TextLocation, error, error_raw};
use parser::{Expr, ExprTag, NO_TYPE};
use value::RefStr;
use std::collections::HashMap;

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
  Unit, Float, Bool, Array, Struct(RefStr), Any
}

struct FunctionSignature {
  return_type : Type,
  args : Vec<Type>,
}

struct StructType {
  name : RefStr,
  fields : Vec<(RefStr, Type)>,
}

struct Environment<'l> {
  functions : &'l mut HashMap<RefStr, FunctionSignature>,
  structs : &'l mut HashMap<RefStr, StructType>,
  locals : Vec<Vec<(RefStr, Type)>>,
}

impl <'l> Environment <'l> {
  fn new(
    functions : &'l mut HashMap<RefStr, FunctionSignature>,
    structs : &'l mut HashMap<RefStr, StructType>,
    arguments : Vec<(RefStr, Type)>) -> Environment<'l>
  {
    let locals = vec![arguments];
    Environment { functions, structs, locals }
  }

  fn find_var_type(&self, v : &str, loc : &TextLocation) -> Result<Type, Error> {
    for vs in self.locals.iter().rev() {
      for (name, t) in vs.iter().rev() {
        if name.as_ref() == v {
          return Ok(t.clone());
        }
      }
    }
    error(loc, format!("no variable called '{}' found in scope", v))
  }

  fn find_field_type(&self, struct_name : &str, field_name : &str, loc : &TextLocation) -> Result<Type, Error> {
    let def =
      self.structs.get(struct_name)
      .ok_or_else(|| error_raw(*loc, format!("struct {} does not exist", struct_name)))?.clone();
    let (_, field_type) =
      def.fields.iter().find(|(n, _)| n.as_ref() == field_name)
      .ok_or_else(|| error_raw(*loc, format!("field {} does not exist on struct {}", field_name, struct_name)))?;
    Ok(field_type.clone())
  }
}

fn type_unify(a : &Type, b : &Type) -> Option<Type> {
  if a == &Type::Any {
    Some(b.clone())
  }
  else if b == &Type::Any {
    Some(a.clone())
  }
  else if a == b {
    Some(a.clone())
  }
  else {
    None
  }
}

fn types_compatible(a : &Type, b : &Type) -> bool {
  type_unify(a, b).is_some()
}

fn assert_expected_found(expected : &Type, found : &Type, loc : &TextLocation) -> Result<(), Error> {
  if !types_compatible(expected, found) {
    return error(*loc, format!("Expected type {:?}, but found type {:?}.", expected, found));
  }
  return Ok(());
}

fn struct_name(t : Type, loc : &TextLocation) -> Result<RefStr, Error> {
  match t {
    Type::Struct(s) => Ok(s),
    _ => error(loc, format!("Expected a struct but found type {:?}.", t)),
  }
}

fn string_to_type(s : &str, loc : &TextLocation) -> Result<Type, Error> {
  if s == NO_TYPE {
    return Ok(Type::Any);
  }
  match s {
    "float" => Ok(Type::Float),
    "unit" => Ok(Type::Unit),
    "bool" => Ok(Type::Bool),
    "array" => Ok(Type::Array),
    "any" => Ok(Type::Any),
    _ => error(loc, format!("{:?} is not a valid type", s)),
  }
}

fn typecheck_function_call(symbol: &Expr, args: &[(Type, TextLocation)], env : &mut Environment) -> Result<Type, Error> {
  let function_name = symbol.symbol_unwrap()?;
  let def = env.functions.get(function_name)
    .ok_or_else(|| error_raw(symbol, format!("Found no function called '{}'", function_name)))?;
  for (expected_type, (found_type, loc)) in def.args.iter().zip(args) {
    assert_expected_found(expected_type, found_type, &loc)?;
  }
  Ok(def.return_type.clone())
}

fn typecheck_tree(expr : &Expr, env : &mut Environment) -> Result<Type, Error> {
  let instr = expr.tree_symbol_unwrap()?.as_ref();
  let args = expr.children.as_slice();
  match (instr, args) {
    ("call", exprs) => {
      let function_name_expr = &exprs[0];
      let function_name = function_name_expr.symbol_unwrap()?;
      let params =
        exprs[1..].iter().map(
          |arg| Ok((typecheck_env(arg, env)?, arg.loc)))
        .collect::<Result<Vec<(Type, TextLocation)>, Error>>()?;
      match params.as_slice() {
        [(a, a_loc), (b, b_loc)] => {
          if function_name.as_ref() == "==" {
            if !types_compatible(a, b) {
              return error(expr.loc, format!("Expected types {:?} and {:?} to match.", a, b));
            }
            return Ok(Type::Bool);
          }
          let types = match function_name.as_ref() {
            "+" | "-" | "*" | "/" => Some((Type::Float, Type::Float, Type::Float)),
            ">" | "<" | "<=" | ">=" => Some((Type::Float, Type::Float, Type::Bool)),
            "&&" | "||" => Some((Type::Bool, Type::Bool, Type::Bool)),
            _ => None
          };
          if let Some((expected_a, expected_b, t_return)) = types {
            assert_expected_found(&expected_a, a, a_loc)?;
            assert_expected_found(&expected_b, b, b_loc)?;
            return Ok(t_return);
          }
        }
        [(v, v_loc)] => {
          match function_name.as_ref() {
            "-" => { assert_expected_found(&Type::Float, v, v_loc)?; return Ok(Type::Float); }
            "!" => { assert_expected_found(&Type::Bool, v, v_loc)?; return Ok(Type::Bool); }
            _ => {}
          }
        }
        _ => (),
      }
      return typecheck_function_call(function_name_expr, params.as_slice(), env);
    }
    ("block", exprs) => {
      env.locals.push(vec![]);
      let num_exprs = exprs.len();
      if num_exprs > 1 {
        for i in 0..(num_exprs-1) {
          typecheck_env(&exprs[i], env)?;
        }
      }
      let block_type = typecheck_env(&exprs[num_exprs-1], env)?;
      env.locals.pop();
      Ok(block_type)
    }
    ("let", [var_symbol, value_expr]) => {
      let var_symbol = var_symbol.symbol_unwrap()?;
      let value_type = typecheck_env(&value_expr, env)?;
      env.locals.last_mut().unwrap().push((var_symbol.clone(), value_type));
      Ok(Type::Unit)
    }
    ("=", [_, assign_expr, value_expr]) => {
      match &assign_expr.tag {
        ExprTag::Symbol(var_symbol) => {
          let var_type = env.find_var_type(&var_symbol, &assign_expr.loc)?;
          let value_type = typecheck_env(&value_expr, env)?;
          if !types_compatible(&value_type, &var_type) {
            return error(expr, format!("can't assign value of type {:?} to variable of type {:?}", value_type, var_type));
          }
          return Ok(Type::Unit);
        }
        ExprTag::Tree(var_symbol) => {
          match (var_symbol.as_ref(), assign_expr.children.as_slice()) {
            ("index", [array_expr, index_expr]) => {
              let array_type = typecheck_env(array_expr, env)?;
              assert_expected_found(&Type::Array, &array_type, &array_expr.loc)?;
              let index_type = typecheck_env(index_expr, env)?;
              assert_expected_found(&Type::Float, &index_type, &index_expr.loc)?;
              let _value_type = typecheck_env(value_expr, env)?;
              // TODO: array types aren't generic, so we can't check the value being assigned yet
              return Ok(Type::Unit);
            }
            (".", [struct_expr, field_expr]) => {
              let struct_type = typecheck_env(struct_expr, env)?;
              let struct_name = struct_name(struct_type, &struct_expr.loc)?;
              let value_type = typecheck_env(value_expr, env)?;
              let field_type = env.find_field_type(&struct_name, &field_expr.symbol_unwrap()?, &field_expr.loc)?;
              if !types_compatible(&value_type, &field_type) {
                return error(expr, format!("can't assign value of type {:?} to field of type {:?}", value_type, field_type));
              }
              return Ok(Type::Unit);
            }
            _ => (),
          }
        }
        _ => (),
      }
      error(assign_expr, format!("can't assign to {:?}", assign_expr))
    }
    ("if", exprs) => {
      let arg_count = exprs.len();
      if arg_count < 2 || arg_count > 3 {
        return error(expr, "malformed if expression");
      }
      let condition_expr = &exprs[0];
      let condition_type = typecheck_env(condition_expr, env)?;
      assert_expected_found(&Type::Bool, &condition_type, &condition_expr.loc)?;
      let then_type = typecheck_env(&exprs[1], env)?;
      if arg_count == 3 {
        let else_type = typecheck_env(&exprs[2], env)?;
        type_unify(&then_type, &else_type)
          .ok_or_else(||error_raw(expr, format!("if branch types '{:?}' and '{:?}' don't match", then_type, else_type)))
      }
      else {
        Ok(Type::Unit)
      }
    }
    ("struct_define", exprs) => {
      if exprs.len() < 1 {
        return error(expr, "malformed struct definition");
      }
      let name_expr = &exprs[0];
      let name = name_expr.symbol_to_refstr()?;
      if env.structs.contains_key(&name) {
        return error(name_expr, format!("A struct called {} has already been defined", name));
      }
      // TODO: check for duplicates?
      let field_exprs = &exprs[1..];
      let mut fields = vec![];
      for i in (0..(field_exprs.len()-1)).step_by(2) {
        let name = field_exprs[i].symbol_to_refstr()?;
        let type_expr = &field_exprs[i+1];
        let field_type = string_to_type(type_expr.symbol_unwrap()?, &type_expr.loc)?;
        fields.push((name, field_type));
      }
      let def = StructType { name: name.clone(), fields };
      env.structs.insert(name, def);
      Ok(Type::Unit)
    }
    ("struct_instantiate", exprs) => {
      if exprs.len() < 1 || exprs.len() % 2 == 0 {
        return error(expr, format!("malformed struct instantiation {:?}", exprs));
      }
      let name_expr = &exprs[0];
      let name = name_expr.symbol_to_refstr()?;
      let mut field_type_map = {
        let def =
          env.structs.get(name.as_ref())
          .ok_or_else(|| error_raw(name_expr.loc, format!("struct {} does not exist", name)))?.clone();
        def.fields.iter()
        .map(|(s, t)| (s.clone(), t.clone()))
        .collect::<HashMap<RefStr, Type>>()
      };
      for i in (1..exprs.len()).step_by(2) {
        let field_name_expr = &exprs[i];
        let field_name = field_name_expr.symbol_to_refstr()?;
        let type_expr = &exprs[i+1];
        let type_name = typecheck_env(type_expr, env)?;
        let expected_type = field_type_map.remove(field_name.as_ref())
          .ok_or_else(|| error_raw(field_name_expr.loc, format!("field {} does not exist", field_name)))?;
        assert_expected_found(&expected_type, &type_name, &type_expr.loc)?;
      }
      if field_type_map.len() > 0 {
        return error(expr, format!("Some fields not initialised: {:?}", field_type_map));
      }
      Ok(Type::Struct(name))
    }
    (".", [_, expr, field_name]) => {
      let struct_name = struct_name(typecheck_env(expr, env)?, &expr.loc)?;
      let field_type = env.find_field_type(&struct_name, field_name.symbol_unwrap()?, &field_name.loc)?;
      Ok(field_type)
    }
    ("while", exprs) => {
      if exprs.len() != 2 {
        return error(expr, "malformed while block");
      }
      let condition_expr = &exprs[0];
      let condition_type = typecheck_env(condition_expr, env)?;
      assert_expected_found(&Type::Bool, &condition_type, &condition_expr.loc)?;
      let _body_type = typecheck_env(&exprs[1], env)?;
      Ok(Type::Unit)
    }
    ("fun", [name, args, function_body ]) => {
      let name = name.symbol_unwrap()?;
      let args = args.children.as_slice();
      let mut params = vec![];
      for i in (0..(args.len()-1)).step_by(2) {
        let name = args[i].symbol_to_refstr()?;
        let type_expr = &args[i+1];
        let arg_type = string_to_type(type_expr.symbol_unwrap()?, &type_expr.loc)?;
        params.push((name, arg_type));
      }
      let args = params.iter().map(|(_, t)| t.clone()).collect();
      let mut new_env = Environment::new(&mut env.functions, &mut env.structs, params);
      let return_type = typecheck_env(function_body, &mut new_env)?;
      let signature = FunctionSignature { return_type, args };
      new_env.functions.insert(name.clone(), signature);
      Ok(Type::Unit)
    }
    ("literal_array", exprs) => {
      for e in exprs {
        typecheck_env(e, env)?;
      }
      Ok(Type::Array)
    }
    ("index", exprs) => {
      if let [array_expr, index_expr] = exprs {
        let array_type = typecheck_env(array_expr, env)?;
        assert_expected_found(&Type::Array, &array_type, &array_expr.loc)?;
        let index_type = typecheck_env(index_expr, env)?;
        assert_expected_found(&Type::Float, &index_type, &index_expr.loc)?;
        Ok(Type::Any)
      }
      else {
        return error(expr, format!("index instruction expected 2 arguments. Found {}.", exprs.len()));
      }
    }
    _ => {
      error(expr, format!("instruction '{}' with args {:?} is not supported by the type checker.", instr, args))
    }
  }
}

fn typecheck_env(expr : &Expr, env : &mut Environment) -> Result<Type, Error> {
  match &expr.tag {
    ExprTag::Tree(_) => {
      typecheck_tree(expr, env)
    }
    ExprTag::Symbol(s) => {
      if s.as_ref() == "break" {
        Ok(Type::Unit)
      }
      else {
        env.find_var_type(&s, &expr.loc)
      }
    }
    ExprTag::LiteralFloat(_) => {
      Ok(Type::Float)
    }
    ExprTag::LiteralBool(_) => {
      Ok(Type::Bool)
    }
  }
}

pub fn typecheck(expr : &Expr) -> Result<Type, Error> {
  let mut functions = HashMap::new();
  let mut structs = HashMap::new();
  let mut env = Environment::new(&mut functions, &mut structs, vec![]);
  typecheck_env(expr, &mut env)
}
