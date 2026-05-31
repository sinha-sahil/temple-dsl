use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::de::{self, IntoDeserializer, Visitor};
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Decimal(Decimal),
    Str(SmolStr),
    Arr(Vec<Value>),
    Obj(IndexMap<SmolStr, Value>),
}

impl Value {
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

    pub fn obj<K, V, I>(pairs: I) -> Value
    where
        K: Into<SmolStr>,
        V: Into<Value>,
        I: IntoIterator<Item = (K, V)>,
    {
        Value::Obj(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
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
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Int(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Value {
        Value::Int(i64::from(n))
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Value {
        Value::Int(i64::from(n))
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.into())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s.into())
    }
}
impl From<SmolStr> for Value {
    fn from(s: SmolStr) -> Value {
        Value::Str(s)
    }
}
impl From<Decimal> for Value {
    fn from(d: Decimal) -> Value {
        Value::Decimal(d)
    }
}

impl<V: Into<Value>> From<Vec<V>> for Value {
    fn from(v: Vec<V>) -> Value {
        Value::Arr(v.into_iter().map(Into::into).collect())
    }
}

impl<K: Into<SmolStr>, V: Into<Value>, const N: usize> From<[(K, V); N]> for Value {
    fn from(arr: [(K, V); N]) -> Value {
        Value::Obj(arr.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

impl<K, V> From<HashMap<K, V>> for Value
where
    K: Into<SmolStr>,
    V: Into<Value>,
{
    fn from(m: HashMap<K, V>) -> Value {
        Value::Obj(m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

impl<K, V> From<IndexMap<K, V>> for Value
where
    K: Into<SmolStr>,
    V: Into<Value>,
{
    fn from(m: IndexMap<K, V>) -> Value {
        Value::Obj(m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
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
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Int(i) => visitor.visit_i64(*i),
            // String form so rust_decimal's Deserialize lands cleanly into `Decimal`.
            Value::Decimal(d) => visitor.visit_string(d.to_string()),
            Value::Str(s) => visitor.visit_str(s.as_str()),
            Value::Arr(arr) => visitor.visit_seq(ValueSeqAccess { iter: arr.iter() }),
            Value::Obj(obj) => visitor.visit_map(ValueMapAccess {
                iter: obj.iter(),
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
            Value::Int(i) => visitor.visit_f64(*i as f64),
            Value::Decimal(d) => match d.to_f64() {
                Some(f) => visitor.visit_f64(f),
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
        // MVP: only unit-variant enums via string names.
        match self {
            Value::Str(s) => visitor.visit_enum(s.as_str().into_deserializer()),
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
            Some((k, v)) => {
                self.value = Some(v);
                seed.deserialize(de::value::BorrowedStrDeserializer::new(k.as_str()))
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
