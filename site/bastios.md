# BastiOS

[BastiOS](https://github.com/yukikotani231/bastios) is a small x86-64 kernel
written in Bastio. It is the language's main user and the reason most of the
kernel features in the [language reference](language.md) exist.

It boots under QEMU, from a bootable ISO under BIOS or UEFI, and runs a shell
on the serial console. User programs run in ring 3 in their own address spaces,
on every processor. It has a file system on a virtio or ATA disk that survives
power cuts, virtio-net and e1000 network drivers with IPv4, UDP, TCP and DHCP,
and a Linux system call layer that runs static musl programs, BusyBox `ash`
among them, unmodified.

None of its Bastio code has runtime checks. Bounds, overflow, page ownership,
page-table structure, exclusive device access and lock order are proved when
it is compiled; the assembly and C under `boot/` are the trusted base.

- Source and build instructions: [README](https://github.com/yukikotani231/bastios#readme)
- How it is put together: [architecture overview](https://github.com/yukikotani231/bastios/blob/main/docs/architecture.md)
