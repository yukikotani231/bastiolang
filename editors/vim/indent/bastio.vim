" Bastio: indent by the nesting of braces, parentheses and brackets, as
" bastiofmt does (a statement that continues on the next line is not
" indented further here: run bastiofmt for that).
if exists('b:did_indent')
  finish
endif
let b:did_indent = 1

setlocal indentexpr=GetBastioIndent(v:lnum)
setlocal indentkeys=0{,0},0),0],!^F,o,O
let b:undo_indent = 'setlocal indentexpr< indentkeys<'

if exists('*GetBastioIndent')
  finish
endif

function! GetBastioIndent(lnum) abort
  let prev = prevnonblank(a:lnum - 1)
  if prev == 0
    return 0
  endif
  let ind = indent(prev)
  " Code only: drop a trailing comment before looking at the brackets.
  let p = substitute(getline(prev), '//.*$', '', '')
  if p =~ '[{(\[]\s*$'
    let ind += shiftwidth()
  endif
  if getline(a:lnum) =~ '^\s*[})\]]'
    let ind -= shiftwidth()
  endif
  return ind < 0 ? 0 : ind
endfunction
