# How is Erg code transpiled to Python code?

To be precise, Erg code is transpiled to Python bytecode.
However, since Python bytecode can almost be reconstructed into Python code, the equivalent Python code is used as an example here.
By the way, the example presented here is a low optimization level.
More advanced optimizations eliminate things that don't need to be instantiated.

## Record, Record type

It will be transpiled to a namedtuple.
For namedtuple, see [here](https://docs.python.org/3/library/collections.html#collections.namedtuple).
There is a similar function, dataclass, but dataclass has a slight performance drop due to auto-implementation of `__eq__` and `__hash__`.

```python
employee = Employee.new({.name = "John Smith"; .id = 100})
assert employee.name == "John Smith"
```

```python
employee = NamedTuple(['name', 'id'])('John Smith', 100)
assert employee.name == 'John Smith'
```

It will also be converted to a simple tuple if it can be further optimized.

## Polymorphic Type

> WIPs

## Instant Scope

If no namespace conflicts occur, it will simply be mangled and expanded.
Names such as `x::y` are used in bytecode and cannot be associated with Python code, but if you force it to be expressed, it will be as follows.

```python
x =
    y = 1
    y+1
```

```python
x::y = 1
x = x::y + 1
```

## Visibility

It does nothing for public variables as it is Python's default.
Private variables are handled by mangling.

```python
x = 1
y =
    x = 2
    assert module::x == 2 # assert(...) returns None
```

```python
module::x = 1
y::x = 2
assert module::x == 2
y = None
```

## Patch

```python
func b: Bool =
    Invert = Patch Bool
    Invert.
        invert self = not self
    b.invert()
```

```python
def func(b):
    def Invert::invert(self): return not self
    return Invert::invert(b)
```
