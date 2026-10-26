# Capabilities: touching the machine safely

This is the third part of the [tutorial](tutorial.md). It is about the part of Bastio that exists to write operating systems: how a program reaches the console, memory pages, I/O ports and device registers without any `unsafe` escape hatch, and what the compiler proves about it. It assumes the [first part](tutorial.md) (ownership and contracts) and, for the device examples, the [second](tutorial-bits.md) (sized integers).

Bastio has no effect system in the style of Haskell or Koka, and no `unsafe` blocks. It has one mechanism that does the job of both: **capabilities**. A function can do to the machine only what the capabilities passed to it allow, and a capability is a linear value, so the rules of ownership (use exactly once, no copies, no use after it is gone) become rules about hardware.

As elsewhere, every `bastio` block is compiled by `tests/run.sh`; its first line says what must happen. The hosted target simulates pages, ports and device registers with ordinary memory, so most examples below run on your machine; programs for the `kernel` target are only checked (`// expect-ok`).

## 1. No capability, no effect

There is no way to write to the console from nowhere. `serial_write` takes a `Serial`, a value that stands for the console, and the only place a `Serial` comes from is the parameter list of `main`:

```bastio
// expect-exit: 0
fn greet(con: &mut Serial) {
  serial_write(con, 104);     // 'h'
  serial_write(con, 105);     // 'i'
  serial_write(con, 10);
}

fn main(con: &mut Serial) -> u64 {
  greet(&mut con);
  return 0;
}
```

`greet` can print because it was handed the console. A function without one cannot, and nothing can conjure one up:

```bastio
// expect-error: E-TYPE
fn helper() {
  serial_write(con, 104);     // there is no `con` here, and no way to make one
}

fn main(con: &mut Serial) -> u64 {
  helper();
  return 0;
}
```

So the signature of a function tells you what it may do. `fn parse(b: &Bytes) -> u64` touches nothing but its argument. `fn driver(ports: &mut Ports)` may claim I/O ports and nothing else. You can audit a kernel by reading where the capabilities go.

What `main` receives is the target's business. A hosted program can take `Serial`, `Pool`, `Ports`, `MmioSpace` and `Host` (files and processes). A kernel is handed the same, plus `Mmu` and `Cpu`. They cannot be created in any other way, and, being linear, they cannot be copied.

| Capability | What it stands for |
| --- | --- |
| `Serial` | the console |
| `Pool` | the physical pages the kernel may hand out |
| `Frame` | one page of memory, owned by exactly one place |
| `PageTable` | a page-table page, which owns whatever its slots point to |
| `Ports`, `PortIo` | the I/O port space, and a claimed range of it |
| `MmioSpace`, `Mmio` | the device-register window, and a claimed block of it |
| `Dma` | a page lent to a device |
| `Mmu`, `Cpu`, `Lease` | loading page tables, running user code, and a processor's hold on the table it runs on (kernel only) |

The full list, with every builtin, is [Kernel capabilities](language.md#kernel-capabilities) in the reference.

## 2. Pages are owned, so they cannot leak or be reused

The physical memory is a `Pool`. `pool_take(&mut pool, i)` hands out page `i` as a `Frame`, zeroed; `pool_give` puts it back. A `Frame` is linear like a `Buf`, which gives you the three guarantees of the first part for pages instead of heap memory.

No leaks: a page that is taken must go somewhere.

```bastio
// expect-error: E-LINEAR-LEAK
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 1 {
    return 1;
  }
  let f = pool_take(&mut pool, 0);
  return 0;                    // `f` is still owned: the page is lost
}
```

No use after free: once the page is given back, the handle is gone.

```bastio
// expect-error: E-LINEAR-USE
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 1 {
    return 1;
  }
  let f = pool_take(&mut pool, 0);
  pool_give(&mut pool, f);
  return frame_id(f);          // `f` was consumed by pool_give
}
```

And, new in this part, no double allocation. The pool *knows which pages it still has*. That is ghost state: it exists only in the proof and costs nothing at run time. `pool_take` requires `pool_has(pool, i)`, and afterwards the pool no longer has page `i`:

```bastio
// expect-error: E-VERIFY-OWN
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 1 {
    return 1;
  }
  let a = pool_take(&mut pool, 0);
  let b = pool_take(&mut pool, 0);   // page 0 was already taken
  pool_give(&mut pool, a);
  pool_give(&mut pool, b);
  return 0;
}
```

Two owners of one page is how kernels get use-after-free and cross-process leaks. Here it is a compile error, with the page number and the line.

A function that works on the pool states what it needs in its contract, like any other function:

```bastio
// expect-exit: 0
/// Takes pages 0 and 1, and gives them back in the other order.
fn swap_roles(pool: &mut Pool)
  requires pool_size(pool) >= 2 && pool_has(pool, 0) && pool_has(pool, 1)
  ensures pool_has(pool, 0) && pool_has(pool, 1)
{
  let a = pool_take(&mut pool, 0);
  let b = pool_take(&mut pool, 1);
  pool_give(&mut pool, b);
  pool_give(&mut pool, a);
}

fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) >= 2 {
    swap_roles(&mut pool);
  }
  return 0;
}
```

The `ensures` line is what lets `main` keep using the pool afterwards: without it, the caller would not know the pool still holds those pages.

## 3. Page tables own what they map

A `PageTable` is a page that owns the pages and lower tables its slots point to. `pt_put_frame` moves a `Frame` into a slot, and from then on the table owns it; the only way to get the page back is `pt_take_frame`. Mapping a page twice is impossible (the frame was moved), unmapping and using it is impossible (it is a different value now), and freeing a table that still maps something does not type-check: `pt_free` requires every slot empty. The example in the reference, [building and tearing down a two-level table](language.md#kernel-capabilities), is the whole life of an address space in 20 lines, with every page accounted for.

The machine-specific part is small. `Mmu` is the right to load a table into CR3 (or TTBR0 on AArch64) and `Cpu` is the right to run user code. They exist only in the kernel target, and a `Lease` records that a processor is running on a table, which is what makes it a compile error to free a table another processor may still be using.

## 4. Devices: ports and registers

A device is reached through a range of I/O ports or a block of memory-mapped registers. You *claim* the range from the space that owns it; the claim is a linear value and the only way to touch the range:

```bastio
// expect-exit: 0
/// A driver for the eight ports of a UART.
struct Uart { io: PortIo }

fn uart_send(u: &mut Uart, b: u8)
  requires port_count(u.io) == 8
  ensures port_count(u.io) == 8
{
  pio_out8(&mut u.io, 0, b);       // offset 0 of 8: proved in range
}

fn main(ports: &mut Ports) -> u64 {
  let mut u = Uart { io: port_claim(&mut ports, 0x3F8, 8) };
  uart_send(&mut u, 65);
  port_release(&mut ports, u.io);
  return 0;
}
```

Three things are proved here. Every access is inside the claimed range (`off + 1 <= port_count(io)`), so a driver for one device cannot poke another. Nobody else can claim the same ports while this driver has them (a second `port_claim(&mut ports, 0x3FC, 1)` inside `main` above would be `E-VERIFY-OWN`). And the driver has to give the ports back or keep them: a claim cannot be dropped on the floor.

Memory-mapped registers follow the same pattern, with one more rule: an access must be aligned to its width, because a misaligned device access is undefined on real hardware.

```bastio
// expect-error: E-VERIFY-BOUNDS
fn main(win: &mut MmioSpace) -> u64 {
  let mut apic = mmio_claim(&mut win, 0xFEE00000, 4096);
  let v = mmio_read32(&mut apic, 0x32);     // 0x32 is not a multiple of 4
  mmio_release(&mut win, apic);
  return cast[u64](v % 1);
}
```

Changing `0x32` to `0x30` makes it pass. The register contents come back as a `u32`, so everything from the [previous part](tutorial-bits.md) applies to decoding them: `cast`, masks and shifts, with the same proofs.

Reads take `&mut`, not `&`, on purpose. Reading a device register can change the device (pop a FIFO, acknowledge an interrupt), so the compiler treats it as the effect it is.

## 5. DMA: the page a device is writing

A device that reads or writes memory on its own is the classic source of kernel memory corruption: the driver frees the page, the device writes into whatever lives there now. Bastio ties the lifetime to the type. To give a page to a device, you turn the `Frame` into a `Dma`; while it is a `Dma`, it is not a `Frame`, and nothing that takes a `Frame` accepts it.

```bastio
// expect-error: E-TYPE
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 1 {
    return 1;
  }
  let d = dma_lend(pool_take(&mut pool, 0));
  pool_give(&mut pool, d);        // a lent page cannot go back to the pool
  return 0;
}
```

To get the page back you call `dma_take`, which puts a memory fence in front, so everything the device did is visible before the page is reused:

```bastio
// expect-exit: 0
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) == 0 {
    return 0;
  }
  let mut d = dma_lend(pool_take(&mut pool, 0));
  let addr = dma_addr(d);               // the address to program into the device
  dma_write64(&mut d, 0, addr);         // fill in a descriptor
  let f = dma_take(d);
  pool_give(&mut pool, f);
  return 0;
}
```

What the compiler cannot know is what the device does with the page; that is the hardware's business, and the reference lists it as part of the [soundness scope](language.md#soundness-scope). What it does know is that the device never touches a page the kernel believes is free.

## 6. A kernel program

Everything above is checked on the `hosted` target by compiling it for your machine. To build for bare metal, pass `--target kernel`: the compiler then writes freestanding C (or assembly) with no C library, and the capabilities become what they are in hardware: a `Frame` is a physical address, a `Serial` is nothing at all, a `PortIo` is a base and a size. This echoes the console until a `q` arrives:

```bastio
// expect-ok
// target: kernel
fn main(con: &mut Serial, mmu: &mut Mmu) {
  mmu_kernel(&mut mmu);
  let mut c = serial_read(con);
  while c != 113
    invariant c <= 255
  {
    serial_write(con, c);
    c = serial_read(con);
  }
}
```

The boot code that calls `main` (a few hundred lines of assembly and C) is the one part you do not write in Bastio. [BastiOS](https://github.com/yukikotani231/bastios), a kernel written in Bastio, supplies it, and the [toolchain page](toolchain.md) lists which code the `--target kernel` output trusts.

## What you have now

- A function's parameters are its permissions. Capabilities are linear, so permissions are not copied or forgotten.
- Ghost state in a capability (which pages the pool holds, which ports are free, how many registers a claim covers) is what the proofs are about, and it costs nothing at run time.
- Devices are modelled only as far as their addresses go. The proofs say a driver stays in its own ports, registers and pages; they do not say the device behaves.

Where next: the [toolchain page](toolchain.md) explains the targets and backends, and [part 4](tutorial-shared.md) shows how locks, atomics and interrupts extend the same ideas to several processors.
