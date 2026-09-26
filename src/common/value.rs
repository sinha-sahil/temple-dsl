use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::de::{self, IntoDeserializer, Visitor};
use smol_str::SmolStr;
use std::collections::HashMap;

/// A dynamic value: a render's input and, unless deserialized into your own
/// type, its output. Numbers are integers or exact decimals, never floats;
/// objects keep insertion order.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)] // variants name themselves
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Decimal(Decimal),
    Str(SmolStr),
    Arr(Vec<Value>),
    Obj(IndexMap<SmolStr, Value>),
}

// Non-recursive: the derived drop overflows the stack on a deep value. Nested
// containers go to a heap worklist; a flat value allocates nothing.
impl Drop for Value {
    #[inline]
    fn drop(&mut self) {
        match self {
            Value::Arr(items) if !items.is_empty() => {}
            Value::Obj(object) if !object.is_empty() => {}
            _ => return,
        }
        let mut stack: Vec<Value> = Vec::new();
        detach_nested(self, &mut stack);
        while let Some(mut value) = stack.pop() {
            detach_nested(&mut value, &mut stack);
        }
    }
}

fn detach_nested(value: &mut Value, stack: &mut Vec<Value>) {
    fn nested(value: &Value) -> bool {
        match value {
            Value::Arr(items) => !items.is_empty(),
            Value::Obj(object) => !object.is_empty(),
            _ => false,
        }
    }
    match value {
        Value::Arr(items) => {
            for child in items.iter_mut().filter(|child| nested(child)) {
                stack.push(std::mem::replace(child, Value::Null));
            }
        }
        Value::Obj(object) => {
            for child in object.values_mut().filter(|child| nested(child)) {
                stack.push(std::mem::replace(child, Value::Null));
            }
        }
        _ => {}
    }
}

impl Value {
    /// The type as a word (`null`, `bool`, `int`, `decimal`, `string`,
    /// `array`, `object`), as `type_of` and error messages print it.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Decimal(_) => "decimal",
            Value::Str(_) => "string",
            Value::Arr(_) => "array",
            Value::Obj(_) => "object",
        }
    }

    /// The items, if this is an array; otherwise the value back.
    pub fn into_arr(mut self) -> Result<Vec<Value>, Value> {
        // taken through `&mut`: `Value` has a `Drop`, so fields can't move out
        match &mut self {
            Value::Arr(items) => Ok(std::mem::take(items)),
            _ => Err(self),
        }
    }

    /// The entries, if this is an object; otherwise the value back.
    pub fn into_obj(mut self) -> Result<IndexMap<SmolStr, Value>, Value> {
        match &mut self {
            Value::Obj(object) => Ok(std::mem::take(object)),
            _ => Err(self),
        }
    }

    /// An object from `(key, value)` pairs.
    ///
    /// ```
    /// use temple_dsl::Value;
    ///
    /// let user = Value::obj([("name", Value::from("Ada")), ("age", Value::Int(36))]);
    /// assert_eq!(user.kind(), "object");
    /// ```
    pub fn obj<K, V, I>(pairs: I) -> Value
    where
        K: Into<SmolStr>,
        V: Into<Value>,
        I: IntoIterator<Item = (K, V)>,
    {
        Value::Obj(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Value {
        Value::Null
    }
}
impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Bool(value)
    }
}
impl From<i64> for Value {
    fn from(value: i64) -> Value {
        Value::Int(value)
    }
}
impl From<i32> for Value {
    fn from(value: i32) -> Value {
        Value::Int(i64::from(value))
    }
}
impl From<u32> for Value {
    fn from(value: u32) -> Value {
        Value::Int(i64::from(value))
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::Str(value.into())
    }
}
impl From<String> for Value {
    fn from(value: String) -> Value {
        Value::Str(value.into())
    }
}
impl From<SmolStr> for Value {
    fn from(value: SmolStr) -> Value {
        Value::Str(value)
    }
}
impl From<Decimal> for Value {
    fn from(value: Decimal) -> Value {
        Value::Decimal(value)
    }
}

impl<V: Into<Value>> From<Vec<V>> for Value {
    fn from(items: Vec<V>) -> Value {
        Value::Arr(items.into_iter().map(Into::into).collect())
    }
}

impl<K: Into<SmolStr>, V: Into<Value>, const N: usize> From<[(K, V); N]> for Value {
    fn from(pairs: [(K, V); N]) -> Value {
        Value::Obj(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

impl<K, V> From<HashMap<K, V>> for Value
where
    K: Into<SmolStr>,
    V: Into<Value>,
{
    fn from(map: HashMap<K, V>) -> Value {
        Value::Obj(
            map.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

impl<K, V> From<IndexMap<K, V>> for Value
where
    K: Into<SmolStr>,
    V: Into<Value>,
{
    fn from(map: IndexMap<K, V>) -> Value {
        Value::Obj(
            map.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

/// Exact: numbers convert from their digits, never through `f64`; a number
/// outside `Decimal`'s range becomes `Null`.
#[cfg(feature = "json")]
impl From<serde_json::Value> for Value {
    fn from(value: serde_json::Value) -> Value {
        match value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(flag) => Value::Bool(flag),
            serde_json::Value::Number(number) => match number.as_i64() {
                Some(int) => Value::Int(int),
                None => number
                    .to_string()
                    .parse()
                    .map(Value::Decimal)
                    .unwrap_or(Value::Null),
            },
            serde_json::Value::String(text) => Value::Str(text.into()),
            serde_json::Value::Array(items) => {
                Value::Arr(items.into_iter().map(Value::from).collect())
            }
            serde_json::Value::Object(object) => Value::Obj(
                object
                    .into_iter()
                    .map(|(key, value)| (SmolStr::new(&key), Value::from(value)))
                    .collect(),
            ),
        }
    }
}

/// Decimals become JSON number tokens with their exact digits.
#[cfg(feature = "json")]
impl From<Value> for serde_json::Value {
    fn from(mut value: Value) -> serde_json::Value {
        match &mut value {
            Value::Null => serde_json::Value::Null,
            Value::Bool(flag) => serde_json::Value::Bool(*flag),
            Value::Int(int) => serde_json::Value::from(*int),
            Value::Decimal(decimal) => decimal
                .to_string()
                .parse::<serde_json::Number>()
                .map(serde_json::Value::Number)
                .unwrap_or_else(|_| serde_json::Value::String(decimal.to_string())),
            Value::Str(text) => serde_json::Value::String(std::mem::take(text).into()),
            Value::Arr(items) => serde_json::Value::Array(
                std::mem::take(items).into_iter().map(Into::into).collect(),
            ),
            Value::Obj(object) => serde_json::Value::Object(
                std::mem::take(object)
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value.into()))
                    .collect(),
            ),
        }
    }
}

impl<'de> de::Deserializer<'de> for &'de Value {
    type Error = de::value::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Value::Null => visitor.visit_unit(),
            Value::Bool(flag) => visitor.visit_bool(*flag),
            Value::Int(int) => visitor.visit_i64(*int),
            // String form so rust_decimal's Deserialize lands cleanly into `Decimal`.
            Value::Decimal(decimal) => visitor.visit_string(decimal.to_string()),
            Value::Str(text) => visitor.visit_str(text.as_str()),
            Value::Arr(items) => visitor.visit_seq(ValueSeqAccess { iter: items.iter() }),
            Value::Obj(object) => visitor.visit_map(ValueMapAccess {
                iter: object.iter(),
                value: None,
            }),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    // An f64 target field is an explicit opt-in; the model itself carries no f64.
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Value::Int(int) => visitor.visit_f64(*int as f64),
            Value::Decimal(decimal) => match decimal.to_f64() {
                Some(float) => visitor.visit_f64(float),
                None => Err(de::Error::custom("decimal out of f64 range")),
            },
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_f64(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        // only unit variants, named by a string
        match self {
            Value::Str(text) => visitor.visit_enum(text.as_str().into_deserializer()),
            other => Err(de::Error::custom(format!(
                "cannot deserialize enum from {}",
                other.kind()
            ))),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 char str string bytes
        byte_buf unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

struct ValueSeqAccess<'a> {
    iter: std::slice::Iter<'a, Value>,
}

impl<'de> de::SeqAccess<'de> for ValueSeqAccess<'de> {
    type Error = de::value::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some(value) => seed.deserialize(value).map(Some),
            None => Ok(None),
        }
    }
}

struct ValueMapAccess<'a> {
    iter: indexmap::map::Iter<'a, SmolStr, Value>,
    value: Option<&'a Value>,
}

impl<'de> de::MapAccess<'de> for ValueMapAccess<'de> {
    type Error = de::value::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                seed.deserialize(de::value::BorrowedStrDeserializer::new(key.as_str()))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        let value = self
            .value
            .take()
            .ok_or_else(|| de::Error::custom("value requested before key"))?;
        seed.deserialize(value)
    }
}
