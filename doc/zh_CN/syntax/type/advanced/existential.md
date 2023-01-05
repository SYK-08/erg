# 存在类型

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/type/advanced/existential.md%26commit_hash%3D44d7784aac3550ba97c8a1eaf20b9264b13d4134)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/type/advanced/existential.md&commit_hash=44d7784aac3550ba97c8a1eaf20b9264b13d4134)

如果存在对应于∀的for-all类型，那么很自然地假设存在对应于∃的存在类型
存在类型并不难。你已经知道存在类型，只是没有意识到它本身

```python
T: Trait
f x: T = ...
```

上面的 trait `T` 被用作存在类型
相比之下，小写的`T`只是一个Trait，`X`是一个for-all类型

```python
f|X <: T| x: X = ...
```

事实上，existential 类型被 for-all 类型所取代。那么为什么会有存在类型这样的东西呢?
首先，正如我们在上面看到的，存在类型不涉及类型变量，这简化了类型规范
此外，由于可以删除类型变量，因此如果它是一个全推定类型，则可以构造一个等级为 2 或更高的类型

```python
show_map f: (|T| T -> T), arr: [Show; _] =
    arr.map x ->
        y = f x
        log y
        y
```

但是，如您所见，existential 类型忘记或扩展了原始类型，因此如果您不想扩展返回类型，则必须使用 for-all 类型
相反，仅作为参数且与返回值无关的类型可以写为存在类型

```python
# id(1): 我希望它是 Int
id|T|(x: T): T = x
# |S <: Show|(s: S) -> () 是多余的
show(s: Show): () = log s
```

顺便说一句，类不称为存在类型。一个类不被称为存在类型，因为它的元素对象是预定义的
存在类型是指满足某种Trait的任何类型，它不是知道实际分配了什么类型的地方。