" Bastio: two-space indentation, `//` comments, `gq` through bastiofmt.
if exists('b:did_ftplugin')
  finish
endif
let b:did_ftplugin = 1

setlocal commentstring=//\ %s
setlocal comments=:///,://
setlocal formatoptions-=t formatoptions+=croql
setlocal shiftwidth=2 softtabstop=2 expandtab
setlocal suffixesadd=.bastio
if executable('bastiofmt')
  setlocal formatprg=bastiofmt\ -
endif

let b:undo_ftplugin = 'setlocal commentstring< comments< formatoptions< shiftwidth< softtabstop< expandtab< suffixesadd< formatprg<'
