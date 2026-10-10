; Highlights for Bastio. Capture names follow the common nvim-treesitter /
; Helix conventions.

(line_comment) @comment

(integer_literal) @number
(boolean_literal) @boolean
(string_literal) @string
(escape_sequence) @string.escape

; Types
(type_identifier) @type
((type_identifier) @type.builtin
  (#any-of? @type.builtin
    "u64" "u32" "u16" "u8" "bool" "Str" "Buf" "Bytes" "Host" "Child" "Serial" "Frame"
    "PageTable" "Pool" "Mmu" "Cpu" "Vec" "Box"))
(variant_identifier) @constructor
(struct_expression name: (identifier) @type)
(variant_expression type: (identifier) @type)
(variant_expression variant: (identifier) @constructor)
(variant_pattern type: (identifier) @type)
(variant_pattern variant: (identifier) @constructor)
(is_expression type: (identifier) @type)
(is_expression variant: (identifier) @constructor)

; Functions
(function_definition name: (identifier) @function)
(spec_function_definition name: (identifier) @function)
(call_expression function: (identifier) @function.call)
((call_expression function: (identifier) @function.builtin)
  (#any-of? @function.builtin "old" "include_bytes"))

; Variables, fields, constants
(parameter name: (identifier) @variable.parameter)
(field_identifier) @property
(const_definition name: (identifier) @constant)
((identifier) @variable.builtin
  (#eq? @variable.builtin "result"))
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

; Keywords
[
  "fn"
  "spec"
  "struct"
  "enum"
  "const"
  "let"
] @keyword

"import" @keyword.import

[
  "if"
  "else"
  "match"
] @keyword.conditional

"while" @keyword.repeat

[
  "return"
  "unreachable"
] @keyword.return

; Proof annotations: they are erased from the generated code.
[
  "requires"
  "ensures"
  "invariant"
  "assert"
  "forall"
] @keyword.directive

"is" @keyword.operator
(mutable_specifier) @keyword.modifier
(wildcard_pattern) @variable.builtin

[
  "+" "-" "*" "/" "%"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "!" "==>"
  "=" "&" "->" "=>"
] @operator

[ "(" ")" "[" "]" "{" "}" ] @punctuation.bracket
[ "," ";" ":" "::" "." ] @punctuation.delimiter
