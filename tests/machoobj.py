#!/usr/bin/env python3
"""Compares a Mach-O relocatable object that bastioc1 writes itself
(stage1/machold.bastio, `mo_rel`) with the one Apple's assembler (or llvm-mc)
writes for the same assembly: the same global symbols, the same read-only and
writable data (the code may differ in the length of a branch, and the
reference resolves some references itself that ours leaves to the linker),
and relocations that are well formed: each names a symbol of ours, a
field inside its section, and a type of the architecture. The programs
tests/macos_lib.sh links with their C harnesses are the oracle for the rest.

usage: tests/machoobj.py OWN [REFERENCE]   (without a reference, only OWN is checked)"""
import struct, sys


def parse(path):
    d = open(path, 'rb').read()
    magic, cpu, _, ftype, ncmds, _, _, _ = struct.unpack('<8I', d[:32])
    assert magic == 0xfeedfacf and ftype == 1, path + ': not a 64-bit Mach-O object'
    off, secs, syms, symtab = 32, [], [], None
    for _ in range(ncmds):
        cmd, size = struct.unpack('<II', d[off:off + 8])
        if cmd == 0x19:
            nsects = struct.unpack('<I', d[off + 64:off + 68])[0]
            for i in range(nsects):
                o = off + 72 + 80 * i
                name = d[o:o + 16].rstrip(b'\0').decode()
                addr, sz, foff, _, reloff, nreloc, flags = struct.unpack('<2Q5I', d[o + 32:o + 68])
                secs.append(dict(name=name, addr=addr, size=sz, off=foff, reloff=reloff, nreloc=nreloc, flags=flags))
        elif cmd == 2:
            symtab = struct.unpack('<4I', d[off + 8:off + 24])
        off += size
    symoff, nsyms, stroff, strsize = symtab
    strs = d[stroff:stroff + strsize]
    for i in range(nsyms):
        strx, ty, sect, desc, val = struct.unpack('<IBBHQ', d[symoff + 16 * i:symoff + 16 * i + 16])
        name = strs[strx:strs.index(b'\0', strx)].decode()
        syms.append(dict(name=name, type=ty, sect=sect, desc=desc, value=val))
    for s in secs:
        s['relocs'] = []
        for i in range(s['nreloc']):
            addr, w = struct.unpack('<iI', d[s['reloff'] + 8 * i:s['reloff'] + 8 * i + 8])
            s['relocs'].append(dict(addr=addr, sym=w & 0xffffff, pcrel=(w >> 24) & 1, length=(w >> 25) & 3,
                                    extern=(w >> 27) & 1, type=w >> 28))
        s['data'] = d[s['off']:s['off'] + s['size']] if s['flags'] & 0xff != 1 else b''
    return cpu, secs, syms


def main(own_path, ref_path):
    ocpu, osecs, osyms = parse(own_path)
    arm = ocpu == 0x0100000c
    if ref_path:
        compare(ocpu, osecs, osyms, *parse(ref_path))
    check(own_path, ocpu, osecs, osyms)
    og = [s for s in osyms if s['type'] & 1 and s['name']]
    print('machoobj.py: %s is well formed (%d global symbols)' % (own_path, len(og)))


def compare(ocpu, osecs, osyms, rcpu, rsecs, rsyms):
    assert ocpu == rcpu, 'different architectures'
    # the global symbols
    og = sorted((s['name'], bool(s['type'] & 0xe)) for s in osyms if s['type'] & 1 and s['name'])
    rg = sorted((s['name'], bool(s['type'] & 0xe)) for s in rsyms if s['type'] & 1 and s['name'])
    # a common symbol is undefined with a size; the reference may spell it so too
    assert og == rg, 'global symbols differ:\n  own only: %s\n  reference only: %s' % (
        sorted(set(og) - set(rg))[:10], sorted(set(rg) - set(og))[:10])
    # sections
    on = [s for s in osecs]
    rn = [s for s in rsecs]
    for name in ('__const', '__data'):
        o = next(s for s in on if s['name'] == name)
        r = next((s for s in rn if s['name'] == name), None)
        assert r is not None, name + ' is missing from the reference'
        assert o['size'] == r['size'], '%s: size %d against %d' % (name, o['size'], r['size'])


def check(own_path, ocpu, osecs, osyms):
    on = osecs
    n = 0
    types = {0x01000007: {1, 2, 3, 6, 7, 8}, 0x0100000c: {2, 3, 4, 5, 6, 10}}[ocpu]
    for o in on:
        last = None
        for r in o['relocs']:
            assert r['type'] in types, '%s: relocation type %d' % (o['name'], r['type'])
            assert 0 <= r['addr'] and r['addr'] + 4 <= o['size'], o['name'] + ': relocation outside the section'
            if r['type'] != 10:
                assert r['extern'] and r['sym'] < len(osyms), o['name'] + ': relocation without a symbol'
            assert last is None or r['addr'] <= last, o['name'] + ': relocations are not in descending order'
            last = r['addr']
            n += 1


main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
