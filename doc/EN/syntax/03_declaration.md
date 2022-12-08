# Declaration

Declaration is the syntax for specifying the type of variable to be used.
Declarations can be made anywhere in the code, but declarations alone do not refer to the variables. They must be initialized.
After the assignment, the declaration can be checked to ensure that the type is compatible with the object to which it is assigned.

```python
i: Int
# Can be declared at the same time as the assignment, like i: Int = 2
i = 2
i: Num
i: Nat
i: -2..2
i: {2}
```

Declaration after assignment is similar to type checking by `assert`, but has the feature that it is checked at compile time.
Type checking by `assert` at runtime can be checked for "may be type Foo", but type checking by `:` at compile time is strict: if the type is not determined to be "type Foo", it will not pass the check and an error will occur.

```python
i = (-1..10).sample!
assert i in Nat # this may pass
i: Int # this will pass
i: Nat # this will not pass (-1 is not an element of Nat)
```

Functions can be declared in 2 different ways.

```python,checker_ignore
f: (x: Int, y: Int) -> Int
f: (Int, Int) -> Int
```

If you declare the argument names explicitly, a type error will result if the names are different at definition time. If you want to give the argument names arbitrary names, you can declare them in the second way. In that case, only the method name and its type will be seen by type checking.

```python,compile_fail
T = Trait {
    .f = (x: Int, y: Int): Int
}

C = Class()
C|<: T|.
    f(a: Int, b: Int): Int = ... # TypeError: `.f` must be type of `(x: Int, y: Int) -> Int`, not `(a: Int, b: Int) -> Int`
```

<p align='center'>
    <a href='./02_name.md'>Previous</a> | <a href='./04_function.md'>Next</a>
</p>
