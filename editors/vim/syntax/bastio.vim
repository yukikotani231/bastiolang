" Vim syntax file
" Language: Bastio
" Follows the lexer of bastioc0 and bastioc1 (stage1/lexer.bastio).

if exists('b:current_syntax')
  finish
endif

syn keyword bastioKeyword fn struct enum spec const let mut export import
syn keyword bastioKeyword is
syn keyword bastioConditional if else match
syn keyword bastioRepeat while
syn keyword bastioStatement return unreachable assert
syn keyword bastioSpec requires ensures invariant forall
syn keyword bastioBoolean true false
syn keyword bastioType u8 u16 u32 u64 i8 i16 i32 i64 bool Str Bytes Vec Box Option Lock Atomic
syn keyword bastioType Serial Pool Frame PageTable Shared Dirty Mmu Cpu Lease Ports PortIo MmioSpace Mmio Dma Host Child
syn keyword bastioBuiltin cast wrap old result

syn match bastioFunction '\<[A-Za-z_][A-Za-z0-9_]*\ze\s*\%(\[[^]]*\]\)\=('
syn match bastioDefName '\<\%(fn\|struct\|enum\|const\)\s\+\zs[A-Za-z_][A-Za-z0-9_]*'
syn match bastioDefName '\<spec\s\+fn\s\+\zs[A-Za-z_][A-Za-z0-9_]*'
syn match bastioPath '\<[A-Z][A-Za-z0-9_]*\ze::'
syn match bastioVariant '::\zs[A-Za-z_][A-Za-z0-9_]*'

syn match bastioNumber '\<[0-9][0-9_]*\>'
syn match bastioNumber '\<0[xX][0-9a-fA-F_]\+\>'
syn match bastioOperator '==>\|=>\|->\|==\|!=\|<=\|>=\|&&\|||\|<<\|>>\|::\|[-+*/%=<>!&|^]'

syn region bastioString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=bastioEscape
syn match bastioEscape contained '\\[nrt0\\"]'

syn keyword bastioTodo contained TODO FIXME XXX NOTE
syn match bastioComment '//.*$' contains=bastioTodo,@Spell
syn match bastioDocComment '///.*$' contains=bastioTodo,@Spell

hi def link bastioKeyword Keyword
hi def link bastioConditional Conditional
hi def link bastioRepeat Repeat
hi def link bastioStatement Statement
hi def link bastioSpec PreProc
hi def link bastioBoolean Boolean
hi def link bastioType Type
hi def link bastioBuiltin Special
hi def link bastioFunction Function
hi def link bastioDefName Function
hi def link bastioPath Type
hi def link bastioVariant Constant
hi def link bastioNumber Number
hi def link bastioOperator Operator
hi def link bastioString String
hi def link bastioEscape SpecialChar
hi def link bastioTodo Todo
hi def link bastioComment Comment
hi def link bastioDocComment SpecialComment

let b:current_syntax = 'bastio'
