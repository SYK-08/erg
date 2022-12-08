# Trait

Trait is a nominal type that adds a type attribute requirement to record types.
It is similar to the Abstract Base Class (ABC) in Python, but with the distinction of being able to perform algebraic operations.

```python
Norm = Trait {.x = Int; .y = Int; .norm = Self.() -> Int}
```

Trait does not distinguish between attributes and methods.

Note that traits can only be declared, not implemented (implementation is achieved by a feature called patching, which will be discussed later).
Traits can be checked for implementation in a class by specifying a partial type.

```python
Point2D <: Norm
Point2D = Class {.x = Int; .y = Int}
Point2D.norm self = self.x**2 + self.y**2
```

Error if the required attributes are not implemented.

```python
Point2D <: Norm # TypeError: Point2D is not a subtype of Norm
Point2D = Class {.x = Int; .y = Int}
```

Traits, like structural types, can apply operations such as composition, substitution, and elimination (e.g. `T and U`). The resulting trait is called an instant trait.

```python
T = Trait {.x = Int}
U = Trait {.y = Int}
V = Trait {.x = Int; y: Int}
assert Structural(T and U) == Structural V
assert Structural(V not U) == Structural T
W = Trait {.x = Ratio}
assert Structural(W) ! = Structural(T)
assert Structural(W) == Structural(T.replace {.x = Ratio})
```

Trait is also a type, so it can be used for normal type specification.

```python
points: [Norm; 2] = [Point2D::new(1, 2), Point2D::new(3, 4)]
assert points.iter().map(x -> x.norm()).collect(Array) == [5, 25].
```

## Trait inclusion

`Subsume` allows you to define a trait that contains a certain trait as a supertype. This is called the __subsumption__ of a trait.
In the example below, `BinAddSub` subsumes `BinAdd` and `BinSub`.
This corresponds to Inheritance in a class, but unlike Inheritance, multiple base types can be combined using `and`. Traits that are partially excluded by `not` are also allowed.

```python
Add R = Trait {
    .AddO = Type
    . `_+_` = Self.(R) -> Self.AddO
}

Sub R = Trait {
    .SubO = Type
    . `_-_` = Self.(R) -> Self.SubO
}

BinAddSub = Subsume Add(Self) and Sub(Self)
```

## Structural Traits

Traits can be structured.

```python
SAdd = Structural Trait {
    . `_+_` = Self.(Self) -> Self
}
# |A <: SAdd| cannot be omitted
add|A <: SAdd| x, y: A = x.`_+_` y

C = Class {i = Int}
C.
    new i = Self.__new__ {i;}
    `_+_` self, other: Self = Self.new {i = self::i + other::i}

assert add(C.new(1), C.new(2)) == C.new(3)
```

Nominal traits cannot be used simply by implementing a request method, but must be explicitly declared to have been implemented.
In the following example, `add` cannot be used with an argument of type `C` because there is no explicit declaration of implementation. It must be `C = Class {i = Int}, Impl := Add`.

```python
Add = Trait {
    .`_+_` = Self.(Self) -> Self
}
# |A <: Add| can be omitted
add|A <: Add| x, y: A = x.`_+_` y

C = Class {i = Int}
C.
    new i = Self.__new__ {i;}
    `_+_` self, other: Self = Self.new {i = self::i + other::i}

add C.new(1), C.new(2) # TypeError: C is not a subclass of Add
# hint: inherit or patch 'Add'
```

Structural traits do not need to be declared for this implementation, but instead type inference does not work. Type specification is required for use.

## Polymorphic Traits

Traits can take parameters. This is the same as for polymorphic types.

```python
Mapper T: Type = Trait {
    .mapIter = {Iterator}
    .map = (self: Self, T -> U) -> Self.MapIter U
}

# ArrayIterator <: Mapper
# ArrayIterator.MapIter == ArrayMapper
# [1, 2, 3].iter(): ArrayIterator Int
# [1, 2, 3].iter().map(x -> "\{x}"): ArrayMapper Str
assert [1, 2, 3].iter().map(x -> "\{x}").collect(Array) == ["1", "2", "3"].
```

## Override in Trait

Derived traits can override the type definitions of the base trait.
In this case, the type of the overriding method must be a subtype of the base method type.

```python
# `Self.(R) -> O` is a subtype of ``Self.(R) -> O or Panic
Div R, O: Type = Trait {
    . `/` = Self.(R) -> O or Panic
}
SafeDiv R, O = Subsume Div, {
    @Override
    . `/` = Self.(R) -> O
}
```

## Implementing and resolving duplicate traits in the API

The actual definitions of `Add`, `Sub`, and `Mul` look like this.

```python
Add R = Trait {
    .Output = Type
    . `_+_` = Self.(R) -> .Output
}
Sub R = Trait {
    .Output = Type
    . `_-_` = Self.(R) -> .Output
}
Mul R = Trait {
    .Output = Type
    . `*` = Self.(R) -> .Output
}
```

`.Output` is duplicated. If you want to implement these multiple traits at the same time, specify the following.

```python
P = Class {.x = Int; .y = Int}
# P|Self <: Add(P)| can be abbreviated to P|<: Add(P)|
P|Self <: Add(P)|.
    Output = P
    `_+_` self, other = P.new {.x = self.x + other.x; .y = self.y + other.y}
P|Self <: Mul(Int)|.
    Output = P
    `*` self, other = P.new {.x = self.x * other; .y = self.y * other}
```

Duplicate APIs implemented in this way are almost always type inferred when used, but can also be resolved by explicitly specifying the type with `||`.

```python
print! P.Output # TypeError: ambiguous type
print! P|<: Mul(Int)|.Output # <class 'P'>
```

## Appendix: Differences from Rust traits

Erg's trait is faithful to the one proposed by [Schärli et al.](https://www.ptidej.net/courses/ift6251/fall06/presentations/061122/061122.doc.pdf).
In order to allow algebraic operations, traits are designed to be unable to have method implementations directory, but can be patched if necessary.

<p align='center'>
    <a href='./02_basic.md'>Previous</a> | <a href='./04_class.md'>Next</a>
</p>
