# Temple — FAQ

Implementation-level questions about how the engine actually works. For the
design see [`DESIGN.md`](DESIGN.md); for the build plan,
[`IMPLEMENTATION.md`](IMPLEMENTATION.md).

---

## How are `{{ … }}` placeholders filled in?

At compile, the output literal becomes a tree of `OutNode`s; each `{{ … }}` is
recorded as a hole with an expression ID. At render, the engine recursively
walks the tree and builds the output `Value`:

```rust
enum OutNode {
    Literal(Value),                   // "POST", 3, true — no holes
    Hole(ExprId),                     // {{ expr }} as a value
    Object(Vec<(SmolStr, NodeId)>),   // { "k": <node>, ... }
    Array(Vec<NodeId>),               // [ <node>, ... ] — literal brackets in the template
    Interp(Vec<StrPart>),             // "Hi {{ name }}!"
}
enum StrPart { Text(SmolStr), Hole(ExprId) }

fn build(node: NodeId, ctx: &Ctx) -> Result<Value, RenderError> {
    match &nodes[node] {
        OutNode::Literal(v)   => Ok(v.clone()),
        OutNode::Hole(eid)    => eval_expr(*eid, ctx),
        OutNode::Object(fs)   => Ok(Value::Obj(
            fs.iter().map(|(k, n)| Ok((k.clone(), build(*n, ctx)?))).collect()?
        )),
        OutNode::Array(items) => Ok(Value::Arr(
            items.iter().map(|n| build(*n, ctx)).collect()?
        )),
        OutNode::Interp(ps)   => { /* text parts as-is; holes stringified */ }
    }
}
```

- A **bare hole** emits the expression's typed `Value`.
- A **quoted hole** (interp string) stringifies each hole's value and
  concatenates with the literal text parts.

---

## How does input get parsed, and how is `input.value.nestedValue` resolved?

The caller hands `render` any `impl Into<Value>` — `serde_json::Value`,
`HashMap<String, V>`, or your own struct via `Serialize`. The `From` impls drop
it into Temple's `Value` enum **once**, before render starts. From there it's
just a `Value::Obj(IndexMap<SmolStr, Value>)` sitting under the name `input`.

At parse, `input.value.nestedValue` becomes a root + segment list — built
**once** at compile, not re-parsed per render:

```rust
enum Expr { Path { root: PathRoot, segs: Vec<Seg> }, /* … */ }
enum PathRoot { Input, This, Var(VarId) }
enum Seg { Field(SmolStr), Index(i64), OptField(SmolStr), OptIndex(i64) }
```

At render, walk one segment at a time:

```rust
fn eval_path(root: &Value, segs: &[Seg], ctx: &Ctx) -> Result<Value, RenderError> {
    let mut cur = root;
    for seg in segs {
        cur = match (cur, seg) {
            (Value::Obj(m), Seg::Field(k))     => m.get(k).ok_or_else(|| missing(k, ctx))?,
            (Value::Arr(v), Seg::Index(i))     => v.get(*i as usize).ok_or_else(|| out_of_range(*i, ctx))?,
            (Value::Obj(m), Seg::OptField(k))  => m.get(k).unwrap_or(&Value::Null),
            (Value::Arr(v), Seg::OptIndex(i))  => v.get(*i as usize).unwrap_or(&Value::Null),
            (Value::Null,   Seg::OptField(_) | Seg::OptIndex(_)) => &Value::Null,
            (other, Seg::Field(_) | Seg::Index(_)) => return Err(type_mismatch(other.kind(), ctx)),
            // …
        };
    }
    Ok(cur.clone())
}
```

So `input.value.nestedValue` is just three dictionary lookups on the input
`Value`. No reflection, no string parsing per call. `?.` flips the same lookup
into "missing ⇒ Null, propagate."

---

## How are arrays handled at runtime?

Two distinct cases — important to keep straight:

**(a) Array *literals* in the output template** — when you write:

```
"keys": [{{ input.a }}, {{ input.b }}, 42]
```

This is a fixed-shape, three-element array. The parser builds it as
`OutNode::Array([NodeId(hole_a), NodeId(hole_b), NodeId(literal_42)])`. At
render, each child is built; results collect into a `Value::Arr` of length 3.

**(b) Computed arrays inside an expression** — when you write:

```
"items": {{ input.cart.items.map(it -> it.qty * it.price) }}
```

The output node is a single `OutNode::Hole(eid)`. The expression at `eid` is a
method call:

```rust
enum Expr {
    Method { receiver: ExprId, kind: MethodKind, args: Vec<ExprId> },
    Lambda { params: Vec<VarId>, body: ExprId },
    // …
}
enum MethodKind { Map, Filter, Fold, Len, Sum, Any, All }
```

Evaluation:

```rust
fn eval_method(recv: Value, kind: MethodKind, args: &[ExprId], scope: &mut Scope) -> Result<Value, RenderError> {
    let items = match recv {
        Value::Arr(v) => v,
        other => return Err(method_on_non_array(other)),
    };
    match kind {
        MethodKind::Map => {
            let (param, body) = unwrap_lambda(args[0]);
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                scope.vars[param.0] = it;
                out.push(eval_expr(body, scope)?);
            }
            Ok(Value::Arr(out))
        }
        MethodKind::Filter => { /* same loop, keep where body == Bool(true) */ }
        MethodKind::Fold   => {
            let init = eval_expr(args[0], scope)?;
            let ([acc_p, x_p], body) = unwrap_lambda2(args[1]);
            let mut acc = init;
            for it in items {
                scope.vars[acc_p.0] = acc;
                scope.vars[x_p.0]   = it;
                acc = eval_expr(body, scope)?;
            }
            Ok(acc)
        }
        MethodKind::Len => Ok(Value::Int(items.len() as i64)),
        // sum/any/all — literally fold under the hood
    }
}
```

Lambdas are AST nodes, not values; their params are pre-allocated `VarId`s, so
a call is just "write the param slot, eval the body." No allocation per call.

---

## If my hole evaluates to an array, can I `.map` over it?

**Yes.** The two cases above aren't a constraint — they're how the AST records
*statically-written* arrays differently from *computed* arrays. From the
template author's point of view there's no choice to make: write any
expression in `{{ … }}`, including chained collection methods.

`OutNode::Array([ … ])` exists only when the *template literal* has brackets:
`"keys": [a, b, c]`. The brackets were in your source.

If your hole's expression produces an array, the output node is
`OutNode::Hole(eid)`, and inside the expression you have the full language:

```
"high_value":  {{ input.items.map(it -> it.price).filter(p -> p > 100) }}
"total":       {{ input.items.map(it -> it.qty * it.price).fold(0, (a, b) -> a + b) }}
"first_skus":  {{ input.items.filter(it -> it.in_stock).map(it -> it.sku) }}
```

Each is a single output hole — `OutNode::Hole(eid)` — whose expression yields
a `Value::Arr` of whatever length. The output node doesn't care about shape;
the expression does the work.

The thing you *can't* do is mix the template's bracket syntax with method
syntax outside a hole — e.g. write `[1, 2, 3].map(...)` at the output level.
Methods live in expressions, so the chain has to be inside a `{{ … }}`.

---

## A complete render trace

For `{{ input.cart.items.map(it -> it.qty * it.price).fold(0, (a, b) -> a + b) }}`:

1. The output AST has `Hole(eid)` here. `build` calls `eval_expr(eid)`.
2. `eval_expr` sees `Method { kind: Fold, … }` — evaluate the receiver first.
3. Receiver is `Method { kind: Map, … }` — evaluate *its* receiver first.
4. Receiver of `map` is `Path { Input, [Field("cart"), Field("items")] }` — `eval_path` walks two `.get`s on the input → `Value::Arr(items)`.
5. `.map(it -> it.qty * it.price)` — loop items, bind `it`, evaluate `it.qty * it.price` (two paths + a `Decimal *`) → `Value::Arr(prices)`.
6. `.fold(0, (a, b) -> a + b)` — loop, bind `a, b`, evaluate `a + b`, accumulate → `Value::Decimal(total)`.
7. That `Value` becomes the hole's result. `build` slots it into the output object at the current key.

No reflection, no string lookups, no type erasure. Everything is a `match` on
small enums plus array indexing.
