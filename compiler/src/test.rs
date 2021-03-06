
use crate::error::Error;
use crate::interpret::{Interpreter, interpreter};
use crate::structure::TOP_LEVEL_FUNCTION_NAME;
use crate::compiler::Val;
use crate::c_interface::SStr;

fn result_string(r : Result<Val, Error>) -> String {
  match r {
    Ok(v) => format!("{:?}", v),
    Err(e) => format!("{}", e.display()),
  }
}

fn assert_result_with_interpreter(i : &mut Interpreter, code : &str, expected_result : Val){
  let expected = Ok(expected_result);
  let result = i.eval(code);
  assert!(
    result == expected,
    "error in code '{}'. Expected result '{:?}'. Actual result was '{:?}'",
    code, result_string(expected), result_string(result));
}

fn assert_result(code : &str, expected_result : Val){
  let mut i = interpreter();
  assert_result_with_interpreter(&mut i, code, expected_result)
}

fn assert_error(code : &str, error_substring : &str){
  let mut i = interpreter();
  let result = i.eval(code);
  if let Err(e) = &result {
    let s = format!("{}", e.display());
    if s.contains(error_substring) {
      return; // success
    }
  }
  panic!(
    "error in code '{}'. Expected error containing string '{:?}'. Actual result was '{:?}'",
    code, error_substring, result_string(result));
}

// Runs the tests in isolated processes, because they do unsafe things and could pollute each other.
rusty_fork_test! {
  #[test]
  fn test_basics() {
    let cases = vec![
      ("", Val::Void),
      ("()", Val::Void),
      ("4 + 5", Val::I64(9)),
      ("4. + 5.5", Val::F64(9.5)),
      ("4 - 5", Val::I64(-1)),
      ("4 * 5", Val::I64(20)),
      ("20 > 5", Val::Bool(true)),
      ("20 < 5", Val::Bool(false)),
      ("5 <= 5", Val::Bool(true)),
      ("5 >= 5", Val::Bool(true)),
      ("5 == 5", Val::Bool(true)),
      ("-(4 - 5)", Val::I64(1)),
      ("4 + {let a = 5; let b = 4; a}", Val::I64(9)),
      ("if true then 3 else 4", Val::I64(3)),
      ("if false then 3 else 4", Val::I64(4)),
      ("let a = 5; a", Val::I64(5)),
    ];
    for (code, expected_result) in cases {
      assert_result(code, expected_result);
    }
  }

  #[test]
  fn test_inference() {
    let code = "
      fun blah(a : u64) {
        (a as i64)
      }
      
      fun blah(a : i64) {
        (a as u64)
      }
      
      let a = blah(5)
      let b = a + blah(5)
      let c = b + blah(5)
      
      c
    ";
    assert_result(code, Val::U64(15));
  }


  #[test]
  fn test_conversions() {
    let cases = vec![
      ("4.5 as i32", Val::I32(4)),
      ("4 as u32", Val::U32(4)),
      ("4 as f64", Val::F64(4.0)),
      ("4 as f32", Val::F32(4.0)),
      ("(4 as u32) as i64", Val::I64(4)),
      ("(4 as u32) as u64", Val::U64(4)),
      ("(-4 as i32) as u64", Val::U64((-4 as i32) as u64)),
      ("-4 as u32", Val::U32((-4 as i32) as u32)),
    ];
    for (code, expected_result) in cases {
      assert_result(code, expected_result);
    }
  }

  #[test]
  fn test_and_or() {
    assert_result("true && false", Val::Bool(false));
    assert_result("true || false", Val::Bool(true));
    // Make sure they terminate early
    let and = "
      let a = 0
      false && (a = 1; true)
      a
    ";
    let or = "
      let a = 0
      true || (a = 1; true)
      a
    ";
    assert_result(and, Val::I64(0));
    assert_result(or, Val::I64(0));
  }

  #[test]
  fn test_scope(){
    let code = "
      let a = 4
      let b = if true {
        let a = 5
        a
      }
      else {
        10
      }
      a + b
    ";
    assert_result(code, Val::I64(9));
  }

  #[test]
  fn test_assignment(){
    let a = "
      let a = 4
      a = a + 5
      a
    ";
    let b = "
      struct point {
        x : i64
        y : i64
      }
      let a = point.new(x: 5, y: 50)
      a.x = a.x + 10
      a.y = 500
      a.x + a.y
    ";
    assert_result(a, Val::I64(9));
    assert_result(b, Val::I64(515));
  }

  #[test]
  fn test_struct() {
    let code = "
      struct point {
        x : i64
        y : i64
      }
      fun foo(a : point, b : point) {
        point.new(x: a.x + b.x, y: a.y + b.y)
      }
      let a = point.new(x: 10, y: 1)
      let b = point.new(2, 20)
      let c = foo(a, b)
      c.y
    ";
    assert_result(code, Val::I64(21));
  }

  #[test]
  fn test_struct_in_register() {
    let code = "
      struct point {
        x : i64
        y : ptr(i64)
      }
      fun foo(a : point, b : point) {
        point.new(x: a.x + b.x, y: alloc(*a.y + *b.y))
      }
      let a = point.new(x: 10, y: alloc(1))
      let b = point.new(2, alloc(20))
      *foo(a, b).y
    ";
    assert_result(code, Val::I64(21));
  }

  #[test]
  fn test_union() {
    let a = "
      struct bar {
        a : i32
        b : i32
      }
      union foo {
        u : u64
        i : bar
      }
      let v = foo.new(u : 16 as u64)
      v.i = bar.new(((v.u as i64) + 16) as i32, 0 as i32)
      v.u
    ";
    assert_result(a, Val::U64(32));
    // The expr type returned by "sym" makes use of a union
    let b = "
      let a = {
        let zero = text_marker.new(0, 0)
        let loc = text_location.new(0, zero, zero)
        sym(5, loc)
      }
      //let b = &a
      (*&a).content.data.literal_int
    ";
    assert_result(b, Val::I64(5));
  }

  #[test]
  fn test_return(){
    let code = "
      fun foo(v : bool) {
        if v {
          return 10
        }
        20
      }
      foo(true) + foo(false)
    ";
    assert_result(code, Val::I64(30));
  }

  #[test]
  fn test_load_module(){
    let code = format!(r#"
      let q = #(1 + 1)
      let m = load_module(q).unwrap()
      let f = m.get_function("{}").unwrap() as fun() => i64
      f()
    "#, TOP_LEVEL_FUNCTION_NAME);
    assert_result(code.as_str(), Val::I64(2));
  }

  #[test]
  fn test_quote_interpolation(){
    let a = format!(r#"
      fun test() {{
        let a = #5
        let b = #10
        let q = #($a - $b)
        let m = load_module(q).unwrap()
        let f = m.get_function("{}").unwrap() as fun() => i64
        f()
      }}
      test()
    "#, TOP_LEVEL_FUNCTION_NAME);
    let b = format!(r#"
      fun test(v : i64) {{
        let a = #$v
        let b = #10
        let q = #($a - $b)
        let m = load_module(q).unwrap()
        let f = m.get_function("{}").unwrap() as fun() => i64
        f()
      }}
      test(3)
    "#, TOP_LEVEL_FUNCTION_NAME);

    assert_result(a.as_str(), Val::I64(-5));
    assert_result(b.as_str(), Val::I64(-7));
  }

  #[test]
  fn test_while() {
    let a = "
      let x = 10
      while true {
        x = x - 1
        if x <= 5 {
          break
        }
      }
      x
    ";
    assert_result(a, Val::I64(5));
    let b = "
      let x = 1
      while x < 10 {
        x = x + 6
      }
      x
    ";
    assert_result(b, Val::I64(13));
  }
  
  #[test]
  fn test_for() {
    let a = "
      let x = 0
      for i in range(0, 10) { x = x + i }
      x
    ";
    assert_result(a, Val::I64(45));
    let b = "
      let total = 0
      for x in range(0, 10000) {
        for y in range(10, 20) {
          total = total + x * y
        }
        if x >= 5 {
          break
        }
      }
      total    
    ";
    assert_result(b, Val::I64(2175));
  }


  #[test]
  fn test_jit_module_variable_linking() {
    let mut i = interpreter();
    let a = "static foo = 5";
    let b = "foo";
    assert_result_with_interpreter(&mut i, a, Val::Void);
    assert_result_with_interpreter(&mut i, b, Val::I64(5));
  }

  #[test]
  fn test_jit_module_function_linking() {
    let mut i = interpreter();
    let a = "
      fun foobar() {
        843
      }";
    let b = "foobar()";
    assert_result_with_interpreter(&mut i, a, Val::Void);
    assert_result_with_interpreter(&mut i, b, Val::I64(843));
  }

  #[test]
  fn test_arrays() {
    let code = "
      let a = [0, 1, 2, 3, 6]
      a[1] = 50
      a[1] + a[4] + (a.length as i64)
    ";
    assert_result(code, Val::I64(61));
  }

  #[test]
  fn test_struct_format() {
    let mut i = interpreter();
    #[repr(C)]
    struct Blah {
      x : i32,
      p : *mut i64,
      y : u64,
      z : f32,
    }
    let code = r#"
      struct blah {
        x : i32
        p : ptr(i64)
        y : u64
        z : f32
      }
      fun main(a : ptr(blah)) {
        *a = blah.new(50 as i32, (0 as u64) as ptr(i64), 5390 as u64, 45640.5 as f32)
      }
    "#;
    let b : Blah = i.run_with_pointer_return(code, "main").unwrap();
    assert_eq!(b.x, 50);
    assert_eq!(b.y, 5390);
    assert_eq!(b.z, 45640.5);
  }

  #[test]
  fn test_enum_alignment() {
    let mut i = interpreter();
    #[repr(u8)]
    #[derive(PartialEq, Debug)]
    enum Blah { A(u8), B(i64) }
    let code = r#"
      struct a { tag : u8; data : u8 }
      struct b { tag : u8; data : i64 }
      union blah {
        a : a
        b : b
      }
      fun main(v : ptr(blah)) {
        v[0] = blah.new(a: a.new(0 as u8, 17 as u8))
        v[1] = blah.new(b: b.new(1 as u8, 67))
      }
    "#;
    let blah : [Blah ; 2] = i.run_with_pointer_return(code, "main").unwrap();
    assert_eq!(blah[0], Blah::A(17));
    assert_eq!(blah[1], Blah::B(67));
  }

  /// TODO: test that structs are passed into C functions correctly
  // #[test]
  // fn test_struct_abi() {
  //   The naive approach doesn't work because windows does this:
    
  //       define void @print({ i8*, i64 }* noalias nocapture dereferenceable(16) %s) unnamed_addr #3
    
  //   Incidentally, to trust Godbolt for ABI comparisons on Windows, I have to pass
  //   an argument to rustc to stop it from assuming linux:
    
  //       --target x86_64-pc-windows-msvc
    
  //   panic!("test not implemented");
  // }

  // TODO: this test isn't very good
  #[test]
  fn test_string() {
    let mut i = interpreter();
    let code = r#"
      fun main(a : ptr(string)) {
        *a = "Hello world"
      }
    "#;
    let s : SStr = i.run_with_pointer_return(code, "main").unwrap();
    let expected = "Hello world";
    assert_eq!(s.as_str(), expected);
  }

  #[test]
  fn test_c_function_bind() {
    let code = "
      cbind test_add : fun(a : i64, b : i64) => i64
      test_add(17, 7)
    ";
    assert_result(code, Val::I64(24));
  }

  #[test]
  fn test_c_global_bind() {
    let code = "
      cbind test_global : i64
      test_global
    ";
    assert_result(code, Val::I64(47));
  }

  #[test]
  fn test_overloading() {
    let code = "
      struct foo {}
      struct bar {}

      fun dooby(f : foo) { 10 }
      fun dooby(b : bar) { -20 }

      let f = foo.new()
      let b = bar.new()

      dooby(f) - dooby(b)
    ";
    assert_result(code, Val::I64(30));
  }

  #[test]
  fn test_first_class_function() {
    let code = "
      fun foo(a : i64, b : i64) {
        a + b
      }
      fun fold(a : array(i64), len : i64, v : i64, f : fun(i64, i64) => i64) {
        let i = 0
        while i < len {
          v = f(v, a[i])
          i = i + 1
        }
        v
      }
      let a = [1, 2, 3, 4]
      fold(a, 4, 0, foo)
    ";
    assert_result(code, Val::I64(10));
  }

  #[test]
  fn test_polymorphism() {
    let code = r#"
      let nums = list()

      for x in range(0, 10) {
        nums.add(x)
      }

      let total = 0
      for x in range(0, 10) {
        total = total + nums[x]
      }
      total
    "#;
    assert_result(code, Val::I64(45));
  }

  #[test]
  fn test_index_set() {
    let a = r#"
      let l = list()
      l.add(5)
      l[0] = 4
      l[0]
    "#;
    let b = r#"
      struct blah { x : i64; y : i64 }
      let l = list()
      l.add(blah.new(40, 100))
      l[0].y = 4
      l[0].x + l[0].y
    "#;
    assert_result(a, Val::I64(4));
    assert_result(b, Val::I64(44));
  }

  #[test]
  fn test_nonexistent_types(){
    let code = "
      struct foo {
        data : sijfsiofssdfio
      }
      10
    ";
    assert_error(code, "sijfsiofssdfio");
  }

  #[test]
  fn test_cyclic_structs(){
    let code = "
      struct tree {
        data : string
        children : ptr(tree)
      }
      5
    ";
    assert_result(code, Val::I64(5));
  }

  #[test]
  fn test_local_variable_error_checking() {
    let code = "
      let a : i32 = b
    ";
    assert_error(code, "");
  }

  #[test]
  fn test_llvm_intrinsics() {
    let code = "
      sqrt(16.0) + sin(0.0) + cos(0.0) + floor(1.5)
    ";
    assert_result(code, Val::F64(6.0));
  }

  /// This sometimes failed in code generation, because the functions could be generated in
  /// either order due to iteration over secure hashmaps. If `a` is generated first, the
  /// compiler front-end generates the expected prototype for malloc. However, if `b` is
  /// generated first, a malloc prototype is generated by LLVM to support its instructionset
  /// (in this case it is allocating an array on the heap). This will change when arrays are
  /// stack allocated, but this shouldn't be possible anyway. LLVM wants malloc to take a u32,
  /// while I presumed it should take a u64.
  /// 
  /// Note: this was fixed by binding my own malloc as "malloc64" internally, to prevent the
  /// symbol names from clashing. This is not a great solution as I still don't know why LLVM
  /// uses a 32bit uint.
  #[test]
  fn test_nondeterministic_malloc_bug() {
    let code = "
      fun a() {
        malloc(sizeof(expr)) as ptr(expr)
      }
      fun b(x : i64, y : i64) {
        [x, y]
      }
    ";
    assert_result(code, Val::Void);
  }

  #[test]
  fn test_literal_hardening_bug() {
    let code = "
      fun foo(p : ptr(u8)) {
        p[0] = 65
      }
    ";
    assert_result(code, Val::Void);
  }

  /// The inference engine expects the block to return void, and complains when the
  /// user tries to return something else. This is because the type checker currently
  /// doesn't understand that evaluated values can be implicitly ignored in block scope.
  #[test]
  fn test_implicit_ignore_block_scope_bug() {
    let cases = vec![
      "if true { 3 }",
      "for i in range(0, 10) { 3 }",
      "while false { 3 }",
      "if true { 3 } else {}",
    ];
    for code in cases {
      assert_result(code, Val::Void);
    }
  }

  /// Infers conflicting types because the last line is `return i` instead of `i`,
  /// and the expression `return i` evaluates to void, which defines the type of the
  /// block expression. The type checker doesn't understand that this the return
  /// value of this block is never used, because the code always terminates with a
  /// return command.
  #[test]
  fn test_return_bug() {
    let code = "
    fun foo(i : i64) {
      if i > 50 {
        return 50
      }
      return i
    }
    ";
    // TODO: fix this issue. At the moment it's being left alone, because
    // the problem is easy to work around.
    // assert_result(code, Val::Void);
    let aaa = ();
  }

  #[test]
  fn test_unused_type_var_bug() {
    let code = "
      fun id(t : T) => T with R, T {
        t
      }
      id(4)
    ";
    assert_error(code, "");
  }

  #[test]
  fn test_invalid_field_error() {
    let a = "
      let a = 5
      let b = a.b
    ";
    let b = "
      let a = 5
      let b : f32 = a.b
    ";
    assert_error(a, "");
    assert_error(b, "");
  }

  #[test]
  fn test_duplicate_symbol_error() {
    let code = "
      static BLAH_BLAH : i64 = 5
      static BLAH_BLAH = 10.0
    ";
    assert_error(code, "");
    // TODO: the error message here is terrible, and the problem isn't spotted until codegen
    let aaa = ();
  }

  // #[test]
  // fn test_type_alias() {
  //   let code = "
  //     type int = i32
  //     fun blah(a : int) {
  //       a + 1
  //     }
  //     blah(2)
  //   ";
  //   assert_result(code, Val::I32(3));
  // }

}
