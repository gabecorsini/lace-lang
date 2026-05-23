# Lace EBNF Grammar
## Version 0.1 (Design Phase)

This document defines the formal syntax of Lace using Extended Backus-Naur Form (EBNF).

Convention:
- `[ x ]`   — optional
- `{ x }`   — zero or more repetitions
- `( x | y )` — alternation
- `"x"`     — literal terminal
- `UPPER`   — lexer token defined in the Lexical section

---

## 1. Lexical Elements

```ebnf
(* Identifiers *)
IDENT         = ALPHA { ALPHA | DIGIT | "_" } ;
TYPE_IDENT    = UPPER_ALPHA { ALPHA | DIGIT | "_" } ;
UPPER_ALPHA   = "A" | "B" | ... | "Z" ;
ALPHA         = "a" | "b" | ... | "z" | UPPER_ALPHA | "_" ;
DIGIT         = "0" | "1" | ... | "9" ;

(* Literals *)
INT_LIT       = [ "-" ] DIGIT { DIGIT } ;
FLOAT_LIT     = [ "-" ] DIGIT { DIGIT } "." DIGIT { DIGIT } [ EXP ] ;
EXP           = ("e" | "E") [ "+" | "-" ] DIGIT { DIGIT } ;
STRING_LIT    = '"' { CHAR } '"' ;
CHAR          = any Unicode code point except '"' and '\' | ESCAPE_SEQ ;
ESCAPE_SEQ    = '\' ( '"' | '\' | 'n' | 't' | 'r' | 'u' HEX HEX HEX HEX ) ;
BOOL_LIT      = "true" | "false" ;
HEX           = DIGIT | "a".."f" | "A".."F" ;

(* Duration literals (for tool timeout) *)
DURATION_LIT  = INT_LIT ( "ms" | "s" | "m" | "h" ) ;

(* Comments *)
LINE_COMMENT  = "//" { any char except newline } NEWLINE ;
BLOCK_COMMENT = "/*" { any char } "*/" ;

(* Whitespace — ignored by parser *)
WS            = { " " | "\t" | "\n" | "\r" } ;
```

---

## 2. Top-Level Structure

```ebnf
program          = [ module_decl ] { use_decl } { top_level_item } ;

module_decl      = "module" module_path NEWLINE ;
module_path      = IDENT { "." IDENT } ;

use_decl         = "use" module_path [ "." "{" import_list "}" ] NEWLINE ;
import_list      = IDENT { "," IDENT } ;

top_level_item   = fn_decl
                 | tool_decl
                 | record_decl
                 | enum_decl
                 | type_alias
                 | const_decl
                 | extern_decl ;
```

---

## 3. Declarations

### 3.1 Functions

```ebnf
fn_decl          = { annotation } "fn" IDENT [ generic_params ] "(" [ param_list ] ")" [ "->" type ] effect_ann "{" block "}" ;

param_list       = param { "," param } [ "," ] ;
param            = IDENT ":" type ;

generic_params   = "<" generic_param { "," generic_param } ">" ;
generic_param    = TYPE_IDENT [ ":" trait_bound { "+" trait_bound } ] ;
trait_bound      = TYPE_IDENT ;

effect_ann       = "[" effect { "," effect } "]" ;
effect           = "Pure" | "IO" | "Mut" | "ToolCall" ;
```

### 3.2 Tool Declarations

```ebnf
tool_decl        = { annotation } "tool" IDENT "(" [ tool_param_list ] ")" "->" type
                   { tool_option } ;

tool_param_list  = tool_param { "," tool_param } [ "," ] ;
tool_param       = IDENT ":" type [ "=" expr ] ;

tool_option      = "retries" ":" INT_LIT
                 | "timeout" ":" DURATION_LIT
                 | "mock" ":" IDENT ;
```

### 3.3 Record Declarations

```ebnf
record_decl      = "record" TYPE_IDENT [ generic_params ] "{" { record_field } "}" ;
record_field     = IDENT ":" type "," ;
```

### 3.4 Enum Declarations

```ebnf
enum_decl        = "enum" TYPE_IDENT [ generic_params ] "{" { enum_variant } "}" ;
enum_variant     = TYPE_IDENT [ enum_variant_body ] "," ;
enum_variant_body = "(" type { "," type } ")"          (* tuple variant *)
                  | "{" { record_field } "}" ;           (* struct variant *)
```

### 3.5 Type Aliases

```ebnf
type_alias       = "type" TYPE_IDENT [ generic_params ] "=" type ;
```

### 3.6 Constants

```ebnf
const_decl       = "const" IDENT ":" type "=" expr ;
```

### 3.7 Extern Declarations

```ebnf
extern_decl      = "extern" "fn" IDENT "(" [ param_list ] ")" "->" type effect_ann
                   "from" ":" STRING_LIT ;
```

---

## 4. Types

```ebnf
type             = primitive_type
                 | generic_type
                 | tuple_type
                 | fn_type
                 | dynamic_type
                 | TYPE_IDENT ;

primitive_type   = "Int" | "Float" | "Bool" | "String" | "Bytes" | "Unit" ;

generic_type     = TYPE_IDENT "<" type { "," type } ">" ;
(* Examples: Option<T>, Result<T, E>, List<T>, Map<K, V>,
             Confident<T>, Uncertain<List<T>> *)

tuple_type       = "(" type { "," type } ")" ;

fn_type          = "fn" "(" [ type { "," type } ] ")" "->" type [ effect_ann ] ;

dynamic_type     = "?" ;
```

---

## 5. Expressions

```ebnf
expr             = literal
                 | IDENT
                 | block_expr
                 | if_expr
                 | match_expr
                 | fn_call
                 | method_call
                 | field_access
                 | index_expr
                 | pipeline_expr
                 | binary_expr
                 | unary_expr
                 | closure_expr
                 | record_literal
                 | list_literal
                 | tuple_literal
                 | return_expr
                 | error_prop_expr ;

literal          = INT_LIT | FLOAT_LIT | STRING_LIT | BOOL_LIT ;

block_expr       = "{" block "}" ;

fn_call          = IDENT [ "::" TYPE_IDENT ] "(" [ arg_list ] ")" ;
arg_list         = expr { "," expr } [ "," ] ;

method_call      = expr "." IDENT "(" [ arg_list ] ")" ;

field_access     = expr "." IDENT ;

index_expr       = expr "[" expr "]" ;

pipeline_expr    = expr "|>" expr
                 | expr "|>" fn_call ;

binary_expr      = expr binary_op expr ;
binary_op        = "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">"
                 | "<=" | ">=" | "&&" | "||" | "++" ;
(* "++" is string/list concatenation *)

unary_expr       = unary_op expr ;
unary_op         = "-" | "!" ;

closure_expr     = "|" [ closure_params ] "|" [ "->" type ] [ effect_ann ] "{" block "}" ;
closure_params   = closure_param { "," closure_param } ;
closure_param    = IDENT [ ":" type ] ;

record_literal   = TYPE_IDENT "{" { record_field_init } "}" ;
record_field_init = IDENT ":" expr "," ;

list_literal     = "[" [ expr { "," expr } ] "]" ;

tuple_literal    = "(" expr { "," expr } ")" ;

return_expr      = "return" [ expr ] ;

error_prop_expr  = expr "?" ;
(* Unwraps Ok(v) to v, or short-circuits with Err(e) *)
```

---

## 6. Statements

```ebnf
stmt             = let_stmt
                 | mut_let_stmt
                 | assign_stmt
                 | expr_stmt
                 | for_stmt
                 | while_stmt
                 | pure_block ;

let_stmt         = "let" IDENT [ ":" type ] "=" expr ;
mut_let_stmt     = "mut" "let" IDENT [ ":" type ] "=" expr ;
assign_stmt      = IDENT "=" expr ;           (* only valid for mut bindings *)
expr_stmt        = expr ;
for_stmt         = "for" IDENT "in" expr "{" block "}" ;
while_stmt       = "while" expr "{" block "}" ;
pure_block       = "pure" "{" block "}" ;     (* compile-error if effectful call inside *)

block            = { stmt } [ expr ] ;        (* optional trailing expr is the block's value *)
```

---

## 7. Pattern Matching

```ebnf
match_expr       = "match" expr "{" { match_arm } "}" ;
match_arm        = pattern "=>" expr "," ;

pattern          = wildcard_pat
                 | literal_pat
                 | ident_pat
                 | tuple_pat
                 | enum_pat
                 | record_pat
                 | or_pat ;

wildcard_pat     = "_" ;
literal_pat      = INT_LIT | FLOAT_LIT | STRING_LIT | BOOL_LIT ;
ident_pat        = IDENT ;                    (* binds the matched value *)
tuple_pat        = "(" pattern { "," pattern } ")" ;
enum_pat         = TYPE_IDENT [ "(" pattern { "," pattern } ")" ]
                 | TYPE_IDENT "{" { IDENT ":" pattern "," } "}" ;
record_pat       = TYPE_IDENT "{" { IDENT ":" pattern "," } [ ".." ] "}" ;
or_pat           = pattern "|" pattern ;

(* Special enum patterns for stdlib types *)
(* These desugar to regular enum_pat:
     Some(x)         => Option::Some(x)
     None            => Option::None
     Ok(v)           => Result::Ok(v)
     Err(e)          => Result::Err(e)
     Confident(v)    => Confident::Confident(v)
     Uncertain(vs)   => Uncertain::Uncertain(vs)
*)
```

---

## 8. Annotations

```ebnf
annotation       = "@" IDENT [ "(" annotation_args ")" ] ;
annotation_args  = annotation_arg { "," annotation_arg } ;
annotation_arg   = IDENT ":" ( INT_LIT | STRING_LIT | DURATION_LIT | BOOL_LIT ) ;

(* Built-in annotations:
   @context_bounded(tokens: N)  -- compile-time token budget constraint
   @checkpoint                  -- runtime checkpoint before/after call
   @deprecated(since: "1.0")    -- deprecation notice
*)
```

---

## 9. Conditional Expressions

```ebnf
if_expr          = "if" expr "{" block "}" { "else" "if" expr "{" block "}" } [ "else" "{" block "}" ] ;
```

---

## 10. Operator Precedence (Highest to Lowest)

| Level | Operators           | Associativity |
|-------|---------------------|---------------|
| 9     | unary `-` `!`       | Right         |
| 8     | `*` `/` `%`         | Left          |
| 7     | `+` `-` `++`        | Left          |
| 6     | `<` `>` `<=` `>=`   | Left          |
| 5     | `==` `!=`           | Left          |
| 4     | `&&`                | Left          |
| 3     | `\|\|`              | Left          |
| 2     | `\|>`               | Left          |
| 1     | `?` (postfix)       | Postfix       |
