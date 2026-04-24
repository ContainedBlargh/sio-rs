# SIO Language Manual

SIO is a variant of the MCxxxx assembly language from Shenzhen I/O™, implemented as a concurrent multi-node interpreter. Programs are written in `.sio` files and run from the command line. You can run multiple files at once; inter-node communication is what the channel registers are for.

```
sio file1.sio file2.sio ...
```

---

## Overview

SIO is an assembly-like language. Each line is one instruction. There are no functions — only labels and jumps. The primary working register is `acc` (the accumulator). Execution loops: when the program counter reaches the last instruction, it wraps back to the top.

---

## Values and Types

SIO is dynamically typed. Values can be:

| Type    | Examples                    |
|---------|-----------------------------|
| Integer | `0`, `-42`, `100`           |
| Float   | `3.14`, `-2.5`, `1.0e3`     |
| String  | `"hello"`, `'world'`, `"\n"`|
| Null    | `null`                      |

String literals use `"double"` or `'single'` quotes. Escape sequences: `\n` (newline), `\t` (tab), `\r` (carriage return).

Arithmetic between mixed types coerces automatically. Strings that look like numbers are parsed as numbers when used in numeric context.

---

## Registers

Registers are the only named storage. There are built-in registers and user-declared registers.

### Built-in Registers

| Register | Read                                | Write                                         |
|----------|-------------------------------------|-----------------------------------------------|
| `acc`    | Current accumulator value           | Set accumulator                               |
| `null`   | Returns null                        | Discards written value                        |
| `clk`    | —                                   | Set clock speed in Hz; `-1` = full speed      |
| `stdout` | Read back last written value        | Write to standard output                      |
| `stderr` | —                                   | Write to standard error                       |
| `stdin`  | Read buffered input as string       | Write `N` (int) to read N bytes; write a string pattern to read until that pattern appears |
| `rng`    | Get a random value                  | Seed the generator; type of seed affects type of output |

### Declaring Custom Registers

Custom registers are declared at the top of the file, before any instructions.

```sio
$dat        # a plain register, initialized to null
$x0         # an XBus channel pin (prefix x)
$p0         # a power channel pin (prefix p)
```

Registers whose names begin with `x` are **XBus pins** (synchronous, blocking).
Registers whose names begin with `p` are **power pins** (asynchronous, broadcast).
All other names are **plain registers** (local to this node).

### Memory Registers

Memory registers are declared at the top of the file. They act like arrays.

```sio
*arr[10]    # fixed-size array of 10 elements
*heap       # growable array (no fixed size)
```

Accessing memory:

```sio
mov 3 &arr      # set the offset (index) to 3
mov 99 *arr     # write 99 to arr[3]
mov *arr acc    # read arr[3] into acc
```

`&arr` is the offset register for `*arr`. You read and write it like any register.

Fixed-size arrays use modular (wrapping) indexing. Negative indices count backward from the end.

Growable arrays extend with `null` values when written beyond their current length.

---

## Declarations

All declarations appear before the first instruction. Order matters for pin declarations — it determines port identity when two nodes share a channel.

```sio
$a          # plain register
$b
$x0         # XBus pin
*buf[16]    # fixed memory
*log        # growable memory
```

---

## Instructions

### Data Movement

```
mov src dst
```
Copies the value of `src` into `dst`. `src` and `dst` can be registers, memory references, or literal values (for `src`).

```
swp r1 r2
```
Swaps the values of two registers.

---

### Arithmetic

All arithmetic instructions operate on `acc`.

```
add value       # acc = acc + value
sub value       # acc = acc - value
mul value       # acc = acc * value
div value       # acc = acc / value  (divide by zero gives 0)
```

`value` can be a literal or a register.

For integers, arithmetic wraps on overflow. For floats, standard IEEE 754. For strings:

- `add` concatenates.
- `sub N` (integer N) truncates to N characters.
- `sub str` removes the first occurrence of `str`.
- `mul N` repeats the string N times.

```
inc reg         # reg = reg + 1
dec reg         # reg = reg - 1
```

---

### Bitwise / String Operations

```
not             # bitwise NOT on acc (integers); byte-flip on strings
dgt index       # acc = character/digit at index (0-based, left to right)
dst index value # set character/digit at index to value
```

---

### Type Conversion

```
cst spec
```

Converts `acc` in place according to `spec`:

| Spec   | Effect                                                      |
|--------|-------------------------------------------------------------|
| `"i"`  | Convert to integer (parse string, truncate float)           |
| `0`    | Same as `"i"` (integer literal 0 means "convert to int")   |
| `"f"`  | Convert to float                                            |
| `"s"`  | Convert to string                                           |
| `"c"`  | Integer ↔ character (ASCII code ↔ single-char string)       |
| `"iN"` | Parse string in base N (e.g. `"i16"` for hex, `"i2"` for binary) |
| `"rgb"`| Parse `#RRGGBB` hex color into `"R G B"` space-separated string |

Example — convert ASCII code 7 (BEL character) to a string:

```sio
mov 7 acc
cst "c"
mov acc bel
```

Example — parse a binary string:

```sio
mov "100110" acc
cst "i2"
mov acc stdout      # prints 38
```

---

### Control Flow

```
jmp label           # unconditional jump to label
slp duration        # sleep for duration milliseconds
slx reg             # sleep until XBus register reg has data ready
nop                 # no operation
end                 # terminate the program
```

Labels are defined by placing `name:` on its own line (or before an instruction).

```sio
loop:
    sub 1
    tgt acc 0
    + jmp loop
```

---

### Conditional Execution

SIO has test instructions that set a branch state. The next lines prefixed with `+` run if the test passed; lines prefixed with `-` run if it failed.

```
teq left right      # test equal
tgt left right      # test greater than
tlt left right      # test less than
tcp left right      # three-way compare
```

Branch lines:

```
+  instruction      # executes if last test was true
-  instruction      # executes if last test was false
```

A `+` or `-` line can be chained — all consecutive `+` lines share the same condition, as do all consecutive `-` lines. The condition resets when a non-conditional instruction is encountered or a new test runs.

`tcp` (three-way compare) works differently: `+` runs when `left > right`, `-` runs when `left < right`, and neither branch runs when they are equal.

Example — countdown with branch:

```sio
tgt acc 0
+ jmp loop
- jmp finalize
```

---

### Run-Once Instructions

Prefix any instruction with `@` to make it execute only on the first pass through the program. After the first execution, that line is permanently disabled.

```sio
@mov -1 clk         # set clock to full speed, once
@mov "Starting\n" stdout
```

This is useful for initialization that should not repeat when the program counter wraps.

---

### Hardware Pin Instructions

```
gen pin on_dur off_dur
```

Generates a timed pulse on a power pin `pin`: hold high for `on_dur` milliseconds, then low for `off_dur` milliseconds.

---

## I/O

### Standard Output

```sio
mov "Hello, world\n" stdout
mov 42 stdout
```

Writes a value to stdout. Any type can be written.

### Standard Input

Read N bytes:

```sio
mov 5 stdin         # request up to 5 bytes
mov stdin acc       # retrieve the buffered input
```

Read until a pattern:

```sio
mov "\n" stdin      # read until newline
mov stdin acc       # acc now holds the line (including the newline)
```

### Standard Error

```sio
mov "error: something went wrong\n" stderr
```

### Random Numbers

```sio
mov null rng        # seed with current time
mov 42 rng          # seed with a specific integer
mov rng acc         # read a random value
```

The type of the seed affects the type of value produced. A string seed produces a random string; an integer seed produces a random integer.

### Commandline Arguments

All commandline arguments that are not `.sio` source files, are added to the special `args` register.
`args` is a fixed-memory register, meaning you can take advantage of the modular indexing to easily get the argument values. `&args` starts out set to the total argument count.

```sio
@mov &args acc      # args were loaded into memory, read initial offset to get amount

mov *args stdout    # &args starts out pointing at the argument count
                    # &args % &args == 0`, thus the first argument is printed.
inc &args           # Increment &args further past the length.
mov "\n" stdout
sub 1               # Count down on `acc`

tlt acc 1           # When acc reaches 0, end the program.
+ end
```

---

## Inter-Node Communication

When you run multiple `.sio` files, they communicate through shared channels. The order of declarations in each file determines which channels they share.

### XBus Pins (synchronous)

XBus pins (`$x0`, `$x1`, ...) synchronize sender and receiver. The sender blocks until the receiver reads; the receiver blocks until the sender writes.

**sender.sio:**
```sio
$x0

@mov -1 clk
@mov 10 acc
mov acc x0          # send; blocks until receiver reads
teq acc -1
+ end
sub 1
```

**receiver.sio:**
```sio
$x0
$sum

slx x0              # wait for data
mov x0 acc          # receive
tlt acc 0
+ mov sum stdout    # print sum when done
+ end
mov sum acc
add acc
mov acc sum
```

Run both together: `sio sender.sio receiver.sio`

### Power Pins (asynchronous)

Power pins (`$p0`, `$p1`, ...) broadcast a value to all readers. The last written value is always available; reads never block.

---

## Memory Arrays

### Fixed-Size Arrays

```sio
*arr[10]

@mov 0 acc

fill:
    mov acc &arr    # set index
    mov acc *arr    # write value at index
    add 1
    tlt acc 10
    + jmp fill
```

### Growable Arrays

```sio
*log

mov 0 &log
mov "first" *log
mov 1 &log
mov "second" *log
```

Indices beyond the current length extend the array with null values.

---

## File I/O

File I/O uses two pairs of registers: `frc`/`frt` (read control/tape) and `fwc`/`fwt` (write control/tape).

### Reading a File

```sio
mov "data.txt" frc      # open file for reading (text mode)
mov frt acc             # read a line
```

Modes (set by writing a string to `frc` after opening):

| Mode  | Meaning             |
|-------|---------------------|
| `"s"` | Text (string) mode  |
| `"i"` | Binary i32 mode     |
| `"f"` | Binary f32 mode     |

Seek by writing an integer to `frc`:

```sio
mov 0 frc               # seek to start
```

### Writing a File

```sio
mov "output.txt" fwc    # open for writing
mov "line one\n" fwt    # write
mov null fwc            # close
```

Write `0` to `fwc` to seek to start; write `-1` to seek to end (append mode).

---

## Graphics

SIO includes a raster graphics system built on `minifb`.

| Register  | Write                                              | Read |
|-----------|----------------------------------------------------|------|
| `xsz`     | Set framebuffer width                              | —    |
| `ysz`     | Set framebuffer height                             | —    |
| `gfx`     | `1` = open/refresh, `0` = refresh only, `-1` = close | —  |
| `*pxl`    | Write pixel value (ARGB integer) at `&pxl` offset | —    |
| `&pxl`    | Set pixel index                                    | —    |
| `kb0`     | —                                                  | Keyboard input (power pin) |

Example — open a 320×240 window:

```sio
mov 320 xsz
mov 240 ysz
mov 1 gfx
```

Then write pixel values to `*pxl` by setting `&pxl` to the pixel index (row × width + column) and writing an ARGB integer to `*pxl`.

---

## Clock Control

```sio
mov 60 clk          # run at 60 Hz
mov -1 clk          # run at full speed (no timing)
```

The clock controls how many times per second the program loops. The default is 1 Hz. Setting `clk` to `-1` disables timing entirely.

---

## Complete Examples

### Hello World

```sio
@mov "Hello, world\n" stdout
end
```

### Fibonacci

```sio
$a
$b
$c
$i
$n

@mov -1 clk
mov 2 i
mov 0 a
mov 1 b

@mov "> " stdout
mov 5 stdin
mov stdin acc
teq acc null
+ end
cst 0
mov acc n
teq n 0
+ jmp ret

loop:
    tgt i n
    + mov b a
    + jmp ret
    mov a acc
    add b
    mov acc c
    mov b a
    mov c b
    mov i acc
    add 1
    mov acc i
    jmp loop

ret:
    mov a stdout
    mov "\n" stdout
```

### Factorial

```sio
$a
$i
$n

@mov -1 clk
mov 1 i
mov 1 a

@mov "> " stdout
mov 5 stdin
mov stdin acc
teq acc "quit\n"
+ end
cst 0
mov acc n
teq n 0
+ jmp ret

loop:
    tgt i n
    + jmp ret
    mov a acc
    mul i
    mov acc a
    mov i acc
    add 1
    mov acc i
    jmp loop

ret:
    mov a stdout
    mov "\n" stdout
```

### Countdown

```sio
@mov -1 clk
@mov 10 acc
@mov "Counting down from 10\n" stdout

loop:
    mov acc stdout
    mov "\n" stdout
    sub 1
    slp 1
tgt acc 0
+ jmp loop
- jmp done

done:
    mov "Done.\n" stdout
    end
```

---

## Quick Reference

### Instructions

| Instruction       | Effect                                      |
|-------------------|---------------------------------------------|
| `mov src dst`     | Copy src to dst                             |
| `swp r1 r2`       | Swap r1 and r2                              |
| `add v`           | acc += v                                    |
| `sub v`           | acc -= v                                    |
| `mul v`           | acc *= v                                    |
| `div v`           | acc /= v                                    |
| `inc r`           | r += 1                                      |
| `dec r`           | r -= 1                                      |
| `not`             | acc = bitwise NOT acc                       |
| `dgt i`           | acc = char/digit at index i of acc          |
| `dst i v`         | set char/digit at index i of acc to v       |
| `cst spec`        | convert acc to type                         |
| `teq a b`         | test equal                                  |
| `tgt a b`         | test greater than                           |
| `tlt a b`         | test less than                              |
| `tcp a b`         | three-way compare (+ = greater, - = lesser) |
| `jmp label`       | jump to label                               |
| `slp n`           | sleep n milliseconds                        |
| `slx reg`         | sleep until XBus reg has data               |
| `gen pin on off`  | pulse power pin                             |
| `nop`             | no operation                                |
| `end`             | exit program                                |

### Modifiers

| Modifier | Meaning                             |
|----------|-------------------------------------|
| `@`      | Run this instruction only once      |
| `+`      | Run if last test passed             |
| `-`      | Run if last test failed             |

### Built-in Registers

| Register | Purpose                        |
|----------|--------------------------------|
| `acc`    | Accumulator                    |
| `null`   | Null sink / null source        |
| `clk`    | Clock speed (Hz, -1 = full)    |
| `stdout` | Standard output                |
| `stderr` | Standard error                 |
| `stdin`  | Standard input                 |
| `rng`    | Random number generator        |
| `frc`    | File read control              |
| `frt`    | File read tape                 |
| `fwc`    | File write control             |
| `fwt`    | File write tape                |
| `gfx`    | Graphics control               |
| `xsz`   | Framebuffer width              |
| `ysz`   | Framebuffer height             |
| `*pxl`  | Pixel buffer                   |
| `&pxl`  | Pixel buffer offset            |
| `kb0`   | Keyboard input (power pin)     |
