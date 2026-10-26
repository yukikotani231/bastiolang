#!/usr/bin/env python3
"""Checks the Mach-O executables bastioc1 writes itself (stage1/machold.bastio)
without a Mac: the header and load commands are consistent with the file, the
segments are page aligned and lie inside it, and, on arm64, the ad-hoc code
signature holds a SHA-256 of every 4096-byte page before it.

usage: tests/macho_check.py FILE..."""
import hashlib, struct, sys


def check(path):
    d = open(path, 'rb').read()
    magic, cpu, _, ftype, ncmds, sizeofcmds, flags, _ = struct.unpack('<8I', d[:32])
    assert magic == 0xfeedfacf and ftype == 2, 'not a 64-bit Mach-O executable'
    arm = cpu == 0x0100000c
    assert arm or cpu == 0x01000007, 'unknown cpu'
    page = 16384 if arm else 4096
    off, sig, segs, main = 32, None, [], None
    for _ in range(ncmds):
        cmd, size = struct.unpack('<II', d[off:off + 8])
        assert size % 8 == 0 and size > 0
        if cmd == 0x19:
            name = d[off + 8:off + 24].rstrip(b'\0').decode()
            vmaddr, vmsize, fileoff, filesize = struct.unpack('<4Q', d[off + 24:off + 56])
            segs.append((name, vmaddr, vmsize, fileoff, filesize))
        elif cmd == 0x1d:
            sig = struct.unpack('<II', d[off + 8:off + 16])
        elif cmd == 0x80000028:
            main = struct.unpack('<Q', d[off + 8:off + 16])[0]
        off += size
    assert off == 32 + sizeofcmds, 'sizeofcmds does not match the commands'
    for name, vmaddr, vmsize, fileoff, filesize in segs:
        if name == '__PAGEZERO':
            continue
        assert vmaddr % page == 0 and fileoff % page == 0, name + ' is not page aligned'
        assert filesize <= vmsize and fileoff + filesize <= len(d), name + ' is outside the file'
    assert segs[-1][0] == '__LINKEDIT' and segs[-1][3] + segs[-1][4] == len(d), '__LINKEDIT does not end the file'
    assert main is not None and main < segs[1][4], 'LC_MAIN is outside __TEXT'
    if arm:
        assert sig is not None and flags & 0x200000, 'arm64 needs a signature and PIE'
        dataoff, datasize = sig
        assert dataoff + datasize == len(d)
        blob = d[dataoff:]
        m, length, count = struct.unpack('>III', blob[:12])
        assert m == 0xfade0cc0 and length == datasize and count == 1
        cd = blob[struct.unpack('>I', blob[16:20])[0]:]
        (cm, cl, ver, fl, hoff, ioff, nspecial, ncode, limit) = struct.unpack('>9I', cd[:36])
        assert cm == 0xfade0c02 and cl == len(cd) and limit == dataoff and nspecial == 0
        assert cd[36] == 32 and cd[37] == 2 and 1 << cd[39] == 4096
        assert ncode == (limit + 4095) // 4096
        for p in range(ncode):
            want = hashlib.sha256(d[p * 4096:min((p + 1) * 4096, limit)]).digest()
            assert cd[hoff + 32 * p:hoff + 32 * p + 32] == want, 'page %d hash' % p
    else:
        assert sig is None


for f in sys.argv[1:]:
    check(f)
    print('macho_check.py: %s ok' % f)
