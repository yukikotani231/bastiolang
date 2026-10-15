# BastiOS against Linux

How fast [BastiOS](bastios.md) is, measured against Linux on the same emulated
machine, running the same programs. Both tables below are reproduced from the
[BastiOS repository](https://github.com/yukikotani231/bastios): `make bench`
times single system calls and file operations, and `make bench-workloads`
times whole programs (gcc, make, apk, tar, gzip, find and grep from Alpine
Linux).

## How it is measured

Both systems boot in QEMU with the same CPU model, memory, processor count and
a virtio disk, and run the same binaries. The host times each test from the
serial line, so the guest's clock cannot skew a result, and a figure is the
median of three runs. Linux is Ubuntu's generic 6.8 kernel with
`mitigations=off`, because BastiOS has no page-table isolation either. On
BastiOS `/disk` is its own disk file system; on Linux it is ext4. The
[performance notes](https://github.com/yukikotani231/bastios/blob/main/docs/performance.md)
describe each test.

The figures come from QEMU's software emulation (TCG) on a four-processor cloud
container, with `-smp 2` and 512 MiB (BastiOS commit `ed8a934`), not from KVM
or from real hardware. Compare the ratios, not the absolute times. The
ratio column is BastiOS's time divided by Linux's, so a figure above 1 means
BastiOS is slower.

## Single operations

| Test | Unit | BastiOS | Linux | BastiOS / Linux |
|---|---|---:|---:|---:|
| cpu | ns/iter | 1.09 | 1.27 | 0.86x |
| memcpy | us/MiB | 54.5 | 61.3 | 0.89x |
| syscall | ns | 16009 | 459 | 34.90x |
| clock | ns | 104 | 999 | 0.10x |
| open@tmp | us | 46.6 | 7.52 | 6.19x |
| open@disk | us | 65.1 | 7.73 | 8.42x |
| stat@tmp | us | 28.6 | 4.83 | 5.91x |
| stat@disk | us | 31.7 | 4.29 | 7.38x |
| create@tmp | us | 108 | 151 | 0.72x |
| create@disk | us | 1326 | 444 | 2.99x |
| filemap@tmp | ns/page | 3800 | 1704 | 2.23x |
| filemap@disk | ns/page | 4246 | 1984 | 2.14x |
| filewrite@tmp | MiB/s | 486.6 | 352.9 | 0.73x |
| filewrite@disk | MiB/s | 76.2 | 202.4 | 2.66x |
| fileread@tmp | MiB/s | 961.3 | 2371.0 | 2.47x |
| fileread@disk | MiB/s | 930.0 | 2595.9 | 2.79x |
| fork | us | 281 | 1259 | 0.22x |
| forkbig | us | 412 | 2177 | 0.19x |
| exec | us | 746 | 4239 | 0.18x |
| spawn | us | 733 | 3645 | 0.20x |
| fault | ns/page | 13612 | 9815 | 1.39x |
| pipelat | us | 62.5 | 97.2 | 0.64x |
| pipebw | MiB/s | 508.8 | 485.1 | 0.95x |
| tcplat | us | 165 | 249 | 0.66x |
| tcpbw | MiB/s | 189.1 | 413.4 | 2.19x |
| sleep | us | 1031 | 1171 | 0.88x |
| epwait | us | 1097 | 1164 | 0.94x |
| idle | % host CPU | 11.3 | 1.00 | 11.34x |

BastiOS is faster at process creation (`fork`, `exec`), pipes, loopback TCP
latency and the clock. It is slower at the system call itself (16 µs against
0.5 µs for `getppid`; the system call path is the largest known gap), at
path lookup (`open`, `stat`), at file I/O on the disk file system and at idle
(11% of a host processor while the guest sleeps, against 1%).

## Whole programs

The same Alpine root and the same Alpine binaries run on both kernels.

| Workload | What it does | BastiOS (s) | Linux (s) | BastiOS / Linux |
|---|---|---:|---:|---:|
| untar | unpack Alpine's mini root (about 500 small files) | 1.08 | 0.22 | 4.96x |
| apk | apk add gcc, make and musl-dev from a local repository | 10.54 | 5.64 | 1.87x |
| hello | gcc -O2 of one small C file, compile and link | 2.45 | 0.55 | 4.49x |
| make1 | make -j1 of a generated project: one gcc -O1 per file, then a link | 71.76 | 79.79 | 0.90x |
| make2 | the same with make -j2 | 39.35 | 41.74 | 0.94x |
| tgz | tar the gcc and header trees through gzip -1 into one file | 4.11 | 1.27 | 3.24x |
| scan | find over the whole root, grep -rl over the headers | 12.79 | 0.27 | 47.54x |

A build of many files is CPU-bound, and it takes about the same time on both
systems (`make1`, `make2`). Work that touches many files or reads a lot
once is where BastiOS is behind: its block cache holds 256 slots, so `scan`,
which walks the whole root, goes back to the disk on every pass. The
[roadmap](roadmap.md) tracks the gaps.

## What is not here

- Real hardware and KVM: the numbers are emulator figures. Once BastiOS runs
  on a physical machine, that table belongs next to Linux's on the same
  machine.
- Network beyond loopback: `tcplat` and `tcpbw` are over 127.0.0.1. A
  comparison over a real virtio-net or e1000 link is not measured yet.
- Native-code backend: the kernel in these tables is built through C. The
  native backend is still slower than that build.
