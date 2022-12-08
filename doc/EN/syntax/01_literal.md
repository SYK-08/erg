# Literal

## Basic Literals

### Int Literal

```python
0, -0, 1, -1, 2, -2, 3, -3, ...
```

### Ratio Literal

```python
0.00, -0.0, 0.1, 400.104, ...
```

Note that the Ratio type is different from the Float type; the API is the same, but there are differences in the accuracy and efficiency of the calculation results.

If a `Ratio` literal has an integer or decimal part of `0`, you can omit the `0`.

```python
assert 1.0 == 1.
assert 0.5 == .5
```

> __Note__: This function `assert` was used to show that `1.0` and `1.` are equal.
Subsequent documents may use `assert` to indicate that the results are equal.

### Str Literal

Any Unicode-representable string can be used.
Unlike Python, quotation marks cannot be enclosed in `'`. If you want to use `"` in a string, use `\"`.

```python
"", "a", "abc", "111", "1# 3f2-3*8$", "こんにちは", "السَّلَامُ عَلَيْكُمْ", ...
```

`\{..}` allows you to embed expressions in strings. This is called string interpolation.
If you want to output `\{..}` itself, use `\\{..}`.

```python
assert "1 + 1 is 2" == "\{1} + \{1} is \{1+1}"
```

### Exponential Literal

This is a literal representing exponential notation often used in academic calculations. It is an instance of type ``Ratio``.
The notation is the same as in Python.

```python
1e-34, 0.4e-10, 2.455+e5, 245e5, 25E5, ...
```

```python
assert 1e-10 == 0.0000000001
```

## Compound Literals

Each of these literals has its own documentation describing them separately, so please refer to that documentation for details.

### [Array Literal](./10_array.md)

```python
[], [1], [1, 2, 3], ["1", "2",], ...
```

### [Tuple Literal](./11_tuple.md)

```python
(), (1, 2, 3), (1, "hello", True), ...
```

### [Dict Literal](./12_dict.md)

```python
{:}, {"one": 1}, {"one": 1, "two": 2}, {"1": 1, "2": 2}, {1: "1", 2: True, "three": [1]}, ...
```

### [Record Literal](./13_record.md)

```python
{=}, {one = 1}, {one = 1; two = 2}, {.name = "John"; .age = 12}, {.name = Str; .age = Nat}, ...
```

### [Set Literal](./14_set.md)

```python
{}, {1}, {1, 2, 3}, {"1", "2", "1"}, ...
```

As a difference from `Array` literals, duplicate elements are removed in `Set`.

```python
assert {1, 2, 1} == {1, 2}
```

## What looks like a literal but isn't

### Boolean Object

`True` and `False` are simply constant objects of type `Bool`.

```python
True, False
```

### None Object

`None` is a singleton object of type `NoneType`.

```python
None
```

### Range Object

Unlike Python's `range`, it can treat not only `Int` but also any object of type that allows comparisons (subtype of `Ord`, e.g. `Str`, `Ratio`, etc.).

```python
assert 0..10 in 5
assert 0..<10 notin 10
assert 0..9 == 0..<10
assert (0..5).to_set() == {1, 2, 3, 4, 5}
assert "a" in "a".."z"
```

### Float Object

```python
assert 0.0f64 == 0
assert 0.0f32 == 0.0f64
```

Float objects are constructed by multiplying a `Ratio` object by `f64`, which is a `Float 64` unit object.

### Complex Object

```python
1+2Im, 0.4-1.2Im, 0Im, Im
```

A `Complex` object is simply an arithmetic combination of an imaginary unit object, `Im`.

### *-less multiplication

In Erg, you can omit the `*` to indicate multiplication as long as there is no confusion in interpretation. However, the combined strength of the operators is set stronger than `*`.

```python
# same as `assert (1*m) / (1*s) == 1*(m/s)`
assert 1m / 1s == 1 (m/s)
```

<p align='center'>
    <a href='./00_basic.md'>Previous</a> | <a href='./02_name.md'>Next</a>
</p>
