# BastiOS

[BastiOS](https://github.com/yukikotani231/bastios) is a small x86-64 kernel
written in Bastio. It is the language's main user and the reason most of the
kernel features in the [language reference](language.md) exist.

It boots under QEMU, from a bootable ISO or USB image under BIOS or UEFI, and
runs a shell on the serial line or the screen. User programs run in ring 3 in
their own address spaces, on every processor. Its disk file system survives
power cuts; drivers cover USB sticks, NVMe, virtio, SATA and ATA disks and
virtio, Intel, Realtek and AWS network cards, under a TCP/IP stack with IPv4,
IPv6, DHCP and DNS. A Linux system call layer runs unmodified Linux programs:
BusyBox, Alpine Linux's binaries and `apk`, Python, Node.js, Go, gcc, Neovim,
`gdb`, and the container runtimes `runc` and `crun`. It also builds for
AArch64 and boots on QEMU's `virt` machine. It is experimental and has only
run in QEMU so far.

None of its Bastio code has runtime checks. Bounds, overflow, page ownership,
page-table structure, exclusive device access and lock order are proved when
it is compiled; the assembly and C under `boot/` are the trusted base.

- Source and build instructions: [README](https://github.com/yukikotani231/bastios#readme)
- How it is put together: [architecture overview](https://github.com/yukikotani231/bastios/blob/main/docs/architecture.md)
