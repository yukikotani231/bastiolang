/**
 * Tree-sitter grammar for Bastio.
 *
 * It follows the recursive-descent parser in stage0/src/parser.rs (whose
 * header comment is the reference grammar) and lexer.rs. It is a little more
 * permissive than stage0 where stage0's checks are not about syntax (empty
 * structs, `import` after a definition, constants that are not `u64`): an
 * editor should still highlight such a file.
 */

const PREC = {
  forall: 0,
  implies: 1,
  or: 2,
  and: 3,
  compare: 4,
  additive: 5,
  multiplicative: 6,
  unary: 7,
  field: 8,
};

const commaSep1 = (rule) => seq(rule, repeat(seq(',', rule)));
const commaSepTrailing = (rule) => optional(seq(commaSep1(rule), optional(',')));

module.exports = grammar({
  name: 'bastio',

  extras: ($) => [/\s/, $.line_comment],

  word: ($) => $.identifier,

  conflicts: ($) => [
    // `if x {` : `x` may be a variable followed by the then-block, or the
    // start of a struct literal `x { field: .. }`. stage0 looks ahead for
    // `{ IDENT :`; here GLR keeps both until one fails. (Names at the head of
    // expressions and patterns are plain identifiers for the same reason: a
    // type, a variant and a variable are not told apart until later tokens.)
    [$._expression, $.struct_expression],
  ],

  rules: {
    source_file: ($) => repeat($._item),

    _item: ($) =>
      choice(
        $.import_declaration,
        $.function_definition,
        $.spec_function_definition,
        $.const_definition,
        $.struct_definition,
        $.enum_definition,
      ),

    // ---- items -----------------------------------------------------------

    import_declaration: ($) => seq('import', field('path', $.string_literal), ';'),

    function_definition: ($) =>
      seq(
        'fn',
        field('name', $.identifier),
        optional(field('type_parameters', $.type_parameters)),
        field('parameters', $.parameters),
        optional(seq('->', field('return_type', $._type))),
        repeat(choice($.requires_clause, $.ensures_clause)),
        field('body', $.block),
      ),

    requires_clause: ($) => seq('requires', $._expression),
    ensures_clause: ($) => seq('ensures', $._expression),

    spec_function_definition: ($) =>
      seq(
        'spec',
        'fn',
        field('name', $.identifier),
        field('parameters', $.parameters),
        '->',
        field('return_type', $._type),
        '{',
        field('body', $._expression),
        '}',
      ),

    const_definition: ($) =>
      seq(
        'const',
        field('name', $.identifier),
        ':',
        field('type', $._type),
        '=',
        field('value', $._expression),
        ';',
      ),

    struct_definition: ($) =>
      seq(
        'struct',
        field('name', $._type_identifier),
        optional(field('type_parameters', $.type_parameters)),
        '{',
        commaSepTrailing($.field_declaration),
        '}',
      ),

    field_declaration: ($) =>
      seq(field('name', alias($.identifier, $.field_identifier)), ':', field('type', $._type)),

    enum_definition: ($) =>
      seq(
        'enum',
        field('name', $._type_identifier),
        optional(field('type_parameters', $.type_parameters)),
        '{',
        commaSepTrailing($.enum_variant),
        '}',
      ),

    enum_variant: ($) =>
      seq(field('name', $._variant_identifier), optional(seq('(', field('payload', $._type), ')'))),

    type_parameters: ($) => seq('[', commaSep1($._type_identifier), ']'),

    parameters: ($) => seq('(', commaSepTrailing($.parameter), ')'),

    parameter: ($) =>
      seq(optional($.mutable_specifier), field('name', $.identifier), ':', field('type', $._type)),

    mutable_specifier: (_) => 'mut',

    // ---- types -----------------------------------------------------------

    _type: ($) => choice($.reference_type, $.generic_type, $._type_identifier),

    reference_type: ($) => seq('&', optional($.mutable_specifier), field('type', $._type)),

    generic_type: ($) =>
      seq(field('type', $._type_identifier), field('type_arguments', $.type_arguments)),

    type_arguments: ($) => seq('[', commaSep1($._type), ']'),

    _type_identifier: ($) => alias($.identifier, $.type_identifier),
    _variant_identifier: ($) => alias($.identifier, $.variant_identifier),

    // ---- statements --------------------------------------------------------

    block: ($) => seq('{', repeat($._statement), '}'),

    _statement: ($) =>
      choice(
        $.let_statement,
        $.assignment_statement,
        $.if_statement,
        $.while_statement,
        $.return_statement,
        $.unreachable_statement,
        $.assert_statement,
        $.match_statement,
        $.expression_statement,
      ),

    let_statement: ($) =>
      seq(
        'let',
        optional($.mutable_specifier),
        field('name', $.identifier),
        optional(seq(':', field('type', $._type))),
        '=',
        field('value', $._expression),
        ';',
      ),

    // stage0 accepts only a place (`x`, `x.f.g`) on the left; any expression
    // is allowed here so that a statement is not ambiguous after its first
    // identifier.
    assignment_statement: ($) =>
      seq(field('left', $._expression), '=', field('right', $._expression), ';'),

    if_statement: ($) =>
      seq(
        'if',
        field('condition', $._expression),
        field('consequence', $.block),
        optional(seq('else', field('alternative', choice($.block, $.if_statement)))),
      ),

    while_statement: ($) =>
      seq(
        'while',
        field('condition', $._expression),
        repeat($.invariant_clause),
        field('body', $.block),
      ),

    invariant_clause: ($) => seq('invariant', $._expression),

    return_statement: ($) => seq('return', optional($._expression), ';'),

    unreachable_statement: (_) => seq('unreachable', ';'),

    assert_statement: ($) => seq('assert', $._expression, ';'),

    match_statement: ($) =>
      seq('match', field('value', $._expression), '{', repeat($.match_arm), '}'),

    match_arm: ($) =>
      seq(field('pattern', $._pattern), '=>', field('body', $.block), optional(',')),

    _pattern: ($) => choice($.wildcard_pattern, $.variant_pattern),

    wildcard_pattern: (_) => '_',

    variant_pattern: ($) =>
      seq(
        field('type', $.identifier),
        optional(field('type_arguments', $.type_arguments)),
        '::',
        field('variant', $.identifier),
        optional(
          seq(
            '(',
            optional($.mutable_specifier),
            field('binding', choice($.identifier, $.wildcard_pattern)),
            ')',
          ),
        ),
      ),

    expression_statement: ($) => seq($._expression, ';'),

    // ---- expressions -------------------------------------------------------

    _expression: ($) =>
      choice(
        $.binary_expression,
        $.unary_expression,
        $.is_expression,
        $.forall_expression,
        $.field_expression,
        $.call_expression,
        $.struct_expression,
        $.variant_expression,
        $.borrow_expression,
        $.parenthesized_expression,
        $.integer_literal,
        $.string_literal,
        $.boolean_literal,
        $.identifier,
      ),

    binary_expression: ($) => {
      const table = [
        [prec.left, PREC.or, '||'],
        [prec.left, PREC.and, '&&'],
        [prec.left, PREC.compare, choice('==', '!=', '<', '<=', '>', '>=')],
        [prec.left, PREC.additive, choice('+', '-')],
        [prec.left, PREC.multiplicative, choice('*', '/', '%')],
      ];
      return choice(
        prec.right(
          PREC.implies,
          seq(field('left', $._expression), field('operator', '==>'), field('right', $._expression)),
        ),
        ...table.map(([fn, p, op]) =>
          fn(p, seq(field('left', $._expression), field('operator', op), field('right', $._expression))),
        ),
      );
    },

    unary_expression: ($) => prec(PREC.unary, seq(field('operator', choice('!', '-')), field('operand', $._expression))),

    is_expression: ($) =>
      prec.left(
        PREC.compare,
        seq(
          field('value', $._expression),
          'is',
          field('type', $.identifier),
          optional(field('type_arguments', $.type_arguments)),
          '::',
          field('variant', $.identifier),
        ),
      ),

    // `forall i: u64 :: body` -- the body extends as far as it can.
    forall_expression: ($) =>
      prec.right(
        PREC.forall,
        seq(
          'forall',
          field('name', $.identifier),
          ':',
          field('type', $._type),
          '::',
          field('body', $._expression),
        ),
      ),

    field_expression: ($) =>
      prec(
        PREC.field,
        seq(field('value', $._expression), '.', field('field', alias($.identifier, $.field_identifier))),
      ),

    call_expression: ($) =>
      seq(
        field('function', $.identifier),
        optional(field('type_arguments', $.type_arguments)),
        field('arguments', $.arguments),
      ),

    arguments: ($) => seq('(', commaSepTrailing($._expression), ')'),

    struct_expression: ($) =>
      seq(
        field('name', $.identifier),
        optional(field('type_arguments', $.type_arguments)),
        '{',
        commaSepTrailing($.field_initializer),
        '}',
      ),

    field_initializer: ($) =>
      seq(field('field', alias($.identifier, $.field_identifier)), ':', field('value', $._expression)),

    variant_expression: ($) =>
      prec.right(
        seq(
          field('type', $.identifier),
          optional(field('type_arguments', $.type_arguments)),
          '::',
          field('variant', $.identifier),
          optional(seq('(', field('payload', $._expression), ')')),
        ),
      ),

    // `&mut place` lends a place; a shared borrow is implicit.
    borrow_expression: ($) =>
      prec.right(seq('&', $.mutable_specifier, field('value', $._place))),

    _place: ($) => choice($.identifier, alias($.place_field, $.field_expression)),

    place_field: ($) =>
      seq(field('value', $._place), '.', field('field', alias($.identifier, $.field_identifier))),

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    // ---- tokens ------------------------------------------------------------

    identifier: (_) => /[\p{L}_][\p{L}\p{N}_]*/u,

    integer_literal: (_) => token(choice(/0[xX][0-9a-fA-F_]+/, /[0-9][0-9_]*/)),

    string_literal: ($) =>
      seq('"', repeat(choice($.string_content, $.escape_sequence)), token.immediate('"')),

    string_content: (_) => token.immediate(prec(1, /[^"\\\n]+/)),

    escape_sequence: (_) => token.immediate(/\\[nrt0\\"]/),

    boolean_literal: (_) => choice('true', 'false'),

    line_comment: (_) => token(seq('//', /[^\n]*/)),
  },
});
