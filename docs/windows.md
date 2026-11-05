# Bastio on Windows

The hosted compilers run on x86-64 Windows. CI builds stage0 and bastioc1 and runs `tests/run.sh` there on every change, with the C backend and with a bastioc1 that the native backend compiled to a Windows executable. This page says how to set it up and what differs from Linux.

## Setting up

The build scripts are shell scripts and the C backend needs a C compiler, so the supported environment is [MSYS2](https://www.msys2.org/) with the MinGW-w64 toolchain. In the **MSYS2 MINGW64** shell:

```sh
pacman -S --needed mingw-w64-x86_64-gcc mingw-w64-x86_64-z3 mingw-w64-x86_64-rust \
    mingw-w64-x86_64-python make diffutils git
git clone https://github.com/yukikotani231/bastiolang && cd bastiolang
(cd stage0 && cargo build --release)
tests/bastioc1.sh
stage0/target/release/bastioc1 -o hello.exe examples/hello.bastio && ./hello.exe
```

`z3` and `cc` must be on `PATH`: the compiler starts both as subprocesses. Programs that bastioc1 itself reads or writes must be named with paths Windows understands (`C:/...`), not MSYS2's `/tmp`; the tests set `TMPDIR=$(cygpath -m /tmp)` for this.

## Two ways to get an executable

- **The C backend** (the default): the generated C is compiled by MinGW-w64's `cc`.
- **The native backend**: `bastioc1 --backend native-ir --os windows -o hello.exe hello.bastio` writes the PE executable itself. The assembler (`stage1/asm86.bastio`, `stage1/asmobj.bastio`) and the PE writer (`stage1/pe.bastio`) are Bastio code, so no GNU `as` or linker is involved. The executable imports from `msvcrt.dll` and `kernel32.dll` (and loads `ws2_32.dll` on the first socket call). `--emit-asm` writes the assembly instead.

A bastioc1 built with the native backend is a normal compiler: it starts `z3` (and `cc` for the C backend) itself, and CI runs the whole test suite with it.

## What differs from Linux

- Child processes start through `CreateProcess` (C backend) or `_spawnvp` (native backend).
- Programs built by the C backend print in text mode, so lines end in CR LF; the native backend writes the bytes it is given.
- `host_net` sockets use Winsock. The connect time limit (`net_connect_timeout`) is ignored. There is no `/etc/resolv.conf`, so `dns_system_resolver` asks PowerShell (`Get-DnsClientServerAddress`) for the machine's first IPv4 DNS server; if PowerShell cannot be started, the lookups that use it (`https_get_by_name`, `https_fetch`) fail, and `dns_lookup_via` with a resolver address the caller supplies still works.
- The sanitizer cross-check of `tests/run.sh` is skipped (MinGW-w64's `gcc` has none), and so is the `--lib` search test.
- `difftest.sh`, `tests/aarch64.sh` and the kernel and user targets are Linux-only.
