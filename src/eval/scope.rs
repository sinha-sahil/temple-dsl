use crate::common::value::Value;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::borrow::Cow;

/// Keys are template or host names, never input data, so a fast unkeyed
/// hash is safe.
pub(crate) type Vars = FxHashMap<SmolStr, Value>;

pub(crate) struct Scope<'a> {
    base: &'a Vars,
    /// A host function body's parameters, looked up after the layers and
    /// before the base map.
    params: Option<Params<'a>>,
    innermost: Option<Layer<'a>>,
}

#[derive(Clone, Copy)]
struct Params<'a> {
    names: &'a [SmolStr],
    values: &'a [Cow<'a, Value>],
}

#[derive(Clone, Copy)]
struct Layer<'a> {
    name: &'a SmolStr,
    value: &'a Value,
    outer: &'a Scope<'a>,
}

impl<'a> Scope<'a> {
    pub(crate) fn base(base: &'a Vars) -> Self {
        Scope {
            base,
            params: None,
            innermost: None,
        }
    }

    pub(crate) fn layered<'b>(&'b self, name: &'b SmolStr, value: &'b Value) -> Scope<'b> {
        Scope {
            base: self.base,
            params: self.params,
            innermost: Some(Layer {
                name,
                value,
                outer: self,
            }),
        }
    }

    pub(crate) fn with_params(
        base: &'a Vars,
        names: &'a [SmolStr],
        values: &'a [Cow<'a, Value>],
    ) -> Scope<'a> {
        Scope {
            base,
            params: Some(Params { names, values }),
            innermost: None,
        }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        let mut scope = self;
        while let Some(layer) = &scope.innermost {
            if layer.name.as_str() == name {
                return Some(layer.value);
            }
            scope = layer.outer;
        }
        if let Some(params) = scope.params {
            // a later parameter shadows an earlier one of the same name
            if let Some(position) = params.names.iter().rposition(|param| param == name) {
                return params.values.get(position).map(|value| &**value);
            }
        }
        scope.base.get(name)
    }
}
