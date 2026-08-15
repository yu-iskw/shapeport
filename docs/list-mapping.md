# Cardinality-preserving list mapping

ShapePort supports deterministic element-wise reshaping of lists through the `listMap` expression in the Transformation Plan IR.

## Invariants

A list map is deliberately cardinality preserving:

```text
List<A> -> List<B>
len(output) == len(input)
order(output) == order(input)
```

A `null` list remains `null`. A `null` element remains a `null` element when nullable elements are allowed by the source and target contracts. The planner does not narrow a nullable list or nullable element contract into a non-nullable target contract automatically.

## Planner inference

The smart planner may infer element-wise mapping for `List<Record<S>> -> List<Record<T>>` only when:

1. the source list field maps unambiguously to the target list field;
2. the source and target nullability relationship is safe;
3. the existing record planner can independently produce a deterministic mapping from `S` to `T`.

If element-level field matching is ambiguous, the entire list mapping remains unresolved. Exact list types are copied directly rather than wrapped in an unnecessary `listMap`.

## Explicitly not inferred

List schemas alone do not establish cardinality-changing intent. ShapePort therefore does **not** infer any of the following from source and target schemas:

- filtering elements;
- exploding elements into output records;
- reducing or aggregating elements;
- joining list elements;
- selecting an arbitrary first/last element.

Those operations require explicit transformation intent.

## Current limitation

The v1alpha1 `listMap` expression takes a `FieldPath` as its input. This supports record-element list mapping but does not yet provide a general current-item expression for recursively synthesizing transformations such as `List<List<A>> -> List<List<B>>`. Such recursive collection semantics should be introduced separately rather than overloaded into the initial operator.
