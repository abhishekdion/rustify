# py2rs — a Python → Rust transpiler

`py2rs` converts a Python project into Rust source. It is written in Rust, has
no external crate dependencies, and reads Python directly — there is no
intermediate step through Python's own `ast` module.

The goal is **native-speed Rust for hot-path code**, not a mechanical
one-to-one translation. `py2rs` infers a Rust type for every expression and
emits idiomatic Rust for the parts it understands; anything it cannot type
confidently is reported as a diagnostic instead of being silently mistranslated.

```
py2rs build --root <project-dir> --entry <file.py> --out <out-dir>
py2rs check <file.py>
py2rs scan  --root <project-dir>
```

## How it works

| Stage | Module | What it does |
|---|---|---|
| Lexer | `src/lexer.rs` | CPython-style tokenizer with `NEWLINE`/`INDENT`/`DEDENT`, implicit string concatenation, f-strings (with `!r` conversions and format specs), bytes literals |
| Parser | `src/parser.rs` | Recursive-descent parser over the supported subset |
| Types | `src/types.rs` | Type lattice (`int`, `float`, `bool`, `str`, `bytes`, `list`, `dict`, `tuple`, `set`, `Optional`, user classes) and the Rust mapping |
| Inference | `src/infer.rs` | Type inference, signature collection, `raises` analysis, mutation analysis |
| Codegen | `src/codegen.rs` | Emits Rust, function by function, driven by the inferred types |
| Runtime | `src/prelude.rs` | The `py2rs_prelude` module: Python semantics Rust's stdlib lacks |

### Type mapping

| Python | Rust |
|---|---|
| `int` | `i64` |
| `float` | `f64` |
| `bool` | `bool` |
| `str` | `String` |
| `bytes` | `Vec<u8>` (borrowed as `&[u8]` for parameters) |
| `list[T]` | `Vec<T>` |
| `dict[K, V]` | `HashMap<K, V>` |
| `tuple[A, B]` | `(A, B)` |
| `Optional[T]` | `Option<T>` |
| `@dataclass` | `struct` with a `Default` impl |
| `class C(B)` | `struct C` with `B`'s fields flattened in |
| `raise` | `Result<T, PyErr>` on the function that raises |

#### New in this slice: Python 3.10 `match` statements

`match` lowers to a native Rust `match`: literal and dotted-constant patterns
(`case Kind.A:`), captures (`case n:`), or-alternatives (`case 2 | 3:`),
guards (`case n if n > 10:`), and tuple unpacking (`case (a, b):`).
Non-exhaustive Python matches get a fallback arm so the generated Rust still
compiles with Python's fall-through semantics, and `match` used as a plain
variable name keeps parsing as an identifier.

#### New in this slice: lambdas

A lambda bound through a `Callable[[..], ..]` annotation gets its full
parameter/return shape from the annotation:

- Module level: `f: Callable[[int, int], int] = lambda a, b: a + b` becomes a
  named `pub fn f(a: i64, b: i64) -> i64 { .. }`.
- Function level: the same binding becomes a Rust closure
  (`let add = |x: i64| (x) + (n);`) that captures surrounding locals naturally.
- A lambda body that can raise becomes `Result<_, PyErr>`, and call sites get
  the `?` propagation.
- `field(default_factory=lambda: [])` keeps working.

Untyped lambdas are reported as diagnostics, not mistranslated.

### Semantic lowering

- **Function scope.** Python locals are visible after the block that assigns
  them. `py2rs` hoists every local to the top of the function body, the way
  CPython does.
- **Exceptions.** A function that can raise becomes `Result<T, PyErr>`.
  `try`/`except` becomes a `match` over a `Result`-returning closure, so the
  happy path stays branch-free.
- **Integer division and modulo.** `//` and `%` use Python's floor semantics
  (`py_floordiv_i64`, `py_mod_i64`), not Rust's truncation.
- **Truthiness.** `if x:` on a list/string/bytes checks emptiness; on an
  `Optional` it checks `is_some`.
- **Indexing.** Negative indices work (`py_bytes_get`, `py_list_get`), and
  slicing follows Python's rules (`py_bytes_slice`, `py_list_slice`).
- **`super()`.** The base-class implementation is kept alongside the
  override as `base_<method>` and `super().m()` calls it.
- **Mutation.** A parameter that the body assigns through is passed as
  `&mut`, so no defensive copies are made.

## Supported subset

Functions, classes, dataclasses, enums, properties, `if`/`elif`/`else`,
`while`, `for`, `break`/`continue`, `try`/`except`/`else`/`finally`, `raise`,
`return`, `with` (the body only), `assert`, `del`, `global`, `nonlocal`,
decorators (`@dataclass`, `@property`, `@abstractmethod`, `@staticmethod`),
comprehensions (list/set/dict), generator expressions, f-strings, `*args`
/`**kwargs` in signatures, keyword arguments, default arguments, nested
functions, closures.

Builtins: `len`, `int`, `float`, `bool`, `str`, `bytes`, `bytearray`, `list`,
`dict`, `tuple`, `set`, `sum`, `min`, `max`, `abs`, `round`, `ord`, `chr`,
`hex`, `bin`, `oct`, `repr`, `range`, `enumerate`, `zip`, `sorted`,
`reversed`, `any`, `all`.

Modules: `struct` (`pack`/`unpack` for the `>H`, `>I`, `>h`, `>i`, `>B`, `>b`
format codes), `datetime.now`, `math`, `enum`, `dataclasses`, `typing`.

Methods on `str`, `bytes`, `list`, `dict` — the common ones.

## Known limitations

These are reported as diagnostics at transpile time, not silently mistranslated:

- `async`/`await`, generators that are consumed lazily.
- Lambdas used in expression position, or bound without a type annotation
  (bind via `f: Callable[[..], ..] = lambda ...`).
- Star-args and double-star-args at call sites.
- Python's arbitrary-precision integers map to `i64`; values beyond that range
  are rejected by the lexer.
- `datetime` becomes an ISO-8601 `String`, so `datetime` arithmetic is not
  supported.
- `dict` iteration order is not preserved (`HashMap` is unordered).

## Building and running

```powershell
cargo build --release
.\target\release\rustify.exe build --root ..\..\Dipulse `
    --entry gateway/app/drivers/bms/jk/parser.py --out .\rust_out
```

### Windows Smart App Control

Smart App Control (WDAC "VerifiedAndReputable") intermittently blocks freshly
linked, unsigned binaries until Defender's cloud reputation service has a
verdict for the new file hash. A clean rebuild produces a different hash that
is accepted. `run.ps1` wraps this: it rebuilds, retries, and falls back to a
clean rebuild when the binary is blocked.

```powershell
.\run.ps1 check ..\..\Dipulse\gateway\app\drivers\bms\jk\frames.py
```

## Layout

Cargo workspace:

```
crates/
  rustify-core/        transpiler library
    src/
      ast.rs           AST
      lexer.rs         tokenizer
      parser.rs        parser
      types.rs         type lattice
      infer.rs         inference and signature collection
      codegen.rs       Rust emitter
      prelude.rs       the runtime emitted next to the generated code
  rustify-cli/         CLI binary (`rustify`)
    src/main.rs        CLI, project discovery, module ordering
```
