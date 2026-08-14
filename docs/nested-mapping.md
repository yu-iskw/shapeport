# Nested structural mapping

ShapePort's deterministic planner supports record-only structural mapping without inferring array semantics.

## Supported

- nested source leaf to flat target, e.g. `customer.name -> customer_name`;
- flat source leaves to nested target records, e.g. `first_name -> person.firstName`;
- nested source to nested target when each target leaf has a unique safe source candidate;
- arbitrarily deep record paths supported by `FieldPath`;
- nested target construction through `Expr::Object`.

Planner explanations identify leaf mappings with full target and source paths.

## Safety rules

Record containers are traversed, but list/map/union values are not recursively interpreted. A list can still be preserved as a whole value when normal type/name matching is safe. ShapePort does not infer `map`, `explode`, array aggregation, or other cardinality-changing semantics from schemas alone.

A scalar source leaf is used at most once per generated plan. Equal high-confidence candidates remain ambiguous. An incompatible nested field remains unresolved rather than being coerced merely because its flattened path resembles the target.

Smart mode can use a normalized full-path match when ordinary leaf-name matching is insufficient. For example, `customer.name` and `customer_name` both normalize to `customername`. This evidence is only accepted when type compatibility and the normal planner ambiguity rules also hold.

## Example

Source contract:

```text
first_name: string
last_name: string
```

Target contract:

```text
person:
  firstName: string
  lastName: string
```

Generated map expression conceptually becomes:

```yaml
person:
  object:
    firstName:
      field: first_name
    lastName:
      field: last_name
```

The existing document VM already evaluates `Expr::Object`, so nested planning does not require a second execution engine.
