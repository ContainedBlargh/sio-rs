# sio-rs

This is a new runtime for [`sio`](https://github.com/ContainedBlargh/sio), implemented with copious amounts of assistance from modern AI tools.

It's got all the things you liked from SIO, plus a few *new features!*

[This time, it comes with a manual.](MANUAL.md)

## Quick Reference

### Instructions

| Instruction      | Effect                                      |
| ---------------- | ------------------------------------------- |
| `mov src dst`    | Copy src to dst                             |
| `swp r1 r2`      | Swap r1 and r2                              |
| `add v`          | acc += v                                    |
| `sub v`          | acc -= v                                    |
| `mul v`          | acc *= v                                    |
| `div v`          | acc /= v                                    |
| `inc r`          | r += 1                                      |
| `dec r`          | r -= 1                                      |
| `not`            | acc = bitwise NOT acc                       |
| `dgt i`          | acc = char/digit at index i of acc          |
| `dst i v`        | set char/digit at index i of acc to v       |
| `cst spec`       | convert acc to type                         |
| `teq a b`        | test equal                                  |
| `tgt a b`        | test greater than                           |
| `tlt a b`        | test less than                              |
| `tcp a b`        | three-way compare (+ = greater, - = lesser) |
| `jmp label`      | jump to label                               |
| `slp n`          | sleep n milliseconds                        |
| `slx reg`        | sleep until XBus reg has data               |
| `gen pin on off` | pulse power pin                             |
| `nop`            | no operation                                |
| `end`            | exit program                                |

### Modifiers

| Modifier | Meaning                             |
|----------|-------------------------------------|
| `@`      | Run this instruction only once      |
| `+`      | Run if last test passed             |
| `-`      | Run if last test failed             |

### Built-in Registers

| Register | Purpose                     |
| -------- | --------------------------- |
| `acc`    | Accumulator                 |
| `null`   | Null sink / null source     |
| `clk`    | Clock speed (Hz, -1 = full) |
| `stdout` | Standard output             |
| `stderr` | Standard error              |
| `stdin`  | Standard input              |
| `rng`    | Random number generator     |
| `frc`    | File read control           |
| `frt`    | File read tape              |
| `fwc`    | File write control          |
| `fwt`    | File write tape             |
| `gfx`    | Graphics control            |
| `xsz`    | Framebuffer width           |
| `ysz`    | Framebuffer height          |
| `*pxl`   | Pixel buffer                |
| `&pxl`   | Pixel buffer offset         |
| `kb0`    | Keyboard input (power pin)  |

