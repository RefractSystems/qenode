// Generated file, do not edit
#![allow(warnings)]
#![allow(clippy::all)]
use serde::{Deserialize, Serialize};

/// Error types.
pub mod error {
    /// Error from a `TryFrom` or `FromStr` implementation.
    pub struct ConversionError(::std::borrow::Cow<'static, str>);
    impl ::std::error::Error for ConversionError {}
    impl ::std::fmt::Display for ConversionError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> Result<(), ::std::fmt::Error> {
            ::std::fmt::Display::fmt(&self.0, f)
        }
    }
    impl ::std::fmt::Debug for ConversionError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> Result<(), ::std::fmt::Error> {
            ::std::fmt::Debug::fmt(&self.0, f)
        }
    }
    impl From<&'static str> for ConversionError {
        fn from(value: &'static str) -> Self {
            Self(value.into())
        }
    }
    impl From<String> for ConversionError {
        fn from(value: String) -> Self {
            Self(value.into())
        }
    }
}
///`Address`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Address.json",
///  "type": "string",
///  "oneOf": [
///    {
///      "type": "integer",
///      "minimum": 0.0
///    },
///    {
///      "type": "string",
///      "pattern": "^(0x[0-9a-fA-F]+|none|sysbus)$"
///    }
///  ],
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum Address {
    Integer(u64),
    String(AddressString),
}
impl ::std::str::FromStr for Address {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        if let Ok(v) = value.parse() {
            Ok(Self::Integer(v))
        } else if let Ok(v) = value.parse() {
            Ok(Self::String(v))
        } else {
            Err("string conversion failed for all variants".into())
        }
    }
}
impl ::std::convert::TryFrom<&str> for Address {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for Address {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for Address {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::fmt::Display for Address {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match self {
            Self::Integer(x) => x.fmt(f),
            Self::String(x) => x.fmt(f),
        }
    }
}
impl ::std::convert::From<u64> for Address {
    fn from(value: u64) -> Self {
        Self::Integer(value)
    }
}
impl ::std::convert::From<AddressString> for Address {
    fn from(value: AddressString) -> Self {
        Self::String(value)
    }
}
///`AddressString`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "type": "string",
///  "pattern": "^(0x[0-9a-fA-F]+|none|sysbus)$"
///}
/// ```
/// </details>
#[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[serde(transparent)]
pub struct AddressString(::std::string::String);
impl ::std::ops::Deref for AddressString {
    type Target = ::std::string::String;
    fn deref(&self) -> &::std::string::String {
        &self.0
    }
}
impl ::std::convert::From<AddressString> for ::std::string::String {
    fn from(value: AddressString) -> Self {
        value.0
    }
}
impl ::std::str::FromStr for AddressString {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        static PATTERN: ::std::sync::LazyLock<::regress::Regex> =
            ::std::sync::LazyLock::new(|| {
                ::regress::Regex::new("^(0x[0-9a-fA-F]+|none|sysbus)$").unwrap()
            });
        if PATTERN.find(value).is_none() {
            return Err("doesn't match pattern \"^(0x[0-9a-fA-F]+|none|sysbus)$\"".into());
        }
        Ok(Self(value.to_string()))
    }
}
impl ::std::convert::TryFrom<&str> for AddressString {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for AddressString {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for AddressString {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl<'de> ::serde::Deserialize<'de> for AddressString {
    fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
    where
        D: ::serde::Deserializer<'de>,
    {
        ::std::string::String::deserialize(deserializer)?
            .parse()
            .map_err(|e: self::error::ConversionError| {
                <D::Error as ::serde::de::Error>::custom(e.to_string())
            })
    }
}
///`Coordinate`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Coordinate.json",
///  "type": "object",
///  "required": [
///    "x",
///    "y",
///    "z"
///  ],
///  "properties": {
///    "x": {
///      "type": "number"
///    },
///    "y": {
///      "type": "number"
///    },
///    "z": {
///      "type": "number"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Coordinate {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Coordinate {
    pub fn builder() -> builder::Coordinate {
        Default::default()
    }
}
///`Cpu`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Cpu.json",
///  "type": "object",
///  "required": [
///    "name",
///    "type"
///  ],
///  "properties": {
///    "isa": {
///      "type": "string"
///    },
///    "memory": {
///      "type": "string"
///    },
///    "mmu_type": {
///      "type": "string"
///    },
///    "name": {
///      "type": "string"
///    },
///    "type": {
///      "type": "string"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Cpu {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub isa: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub memory: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub mmu_type: ::std::option::Option<::std::string::String>,
    pub name: ::std::string::String,
    #[serde(rename = "type")]
    pub type_: ::std::string::String,
}
impl Cpu {
    pub fn builder() -> builder::Cpu {
        Default::default()
    }
}
///`Machine`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Machine.json",
///  "type": "object",
///  "properties": {
///    "cpus": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Cpu"
///      }
///    },
///    "name": {
///      "type": "string"
///    },
///    "type": {
///      "type": "string"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Machine {
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub cpus: ::std::vec::Vec<Cpu>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub name: ::std::option::Option<::std::string::String>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "::std::option::Option::is_none"
    )]
    pub type_: ::std::option::Option<::std::string::String>,
}
impl ::std::default::Default for Machine {
    fn default() -> Self {
        Self {
            cpus: Default::default(),
            name: Default::default(),
            type_: Default::default(),
        }
    }
}
impl Machine {
    pub fn builder() -> builder::Machine {
        Default::default()
    }
}
///`Node`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Node.json",
///  "type": "object",
///  "required": [
///    "name"
///  ],
///  "properties": {
///    "name": {
///      "$ref": "#/$defs/NodeID"
///    },
///    "role": {
///      "anyOf": [
///        {
///          "type": "string",
///          "const": "Cyber"
///        },
///        {
///          "type": "string",
///          "const": "Physics"
///        }
///      ]
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Node {
    pub name: NodeId,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub role: ::std::option::Option<NodeRole>,
}
impl Node {
    pub fn builder() -> builder::Node {
        Default::default()
    }
}
///`NodeId`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "NodeID.json",
///  "type": "string",
///  "oneOf": [
///    {
///      "type": "integer",
///      "minimum": 0.0
///    },
///    {
///      "type": "string"
///    }
///  ],
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum NodeId {
    Integer(u64),
    String(::std::string::String),
}
impl ::std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match self {
            Self::Integer(x) => x.fmt(f),
            Self::String(x) => x.fmt(f),
        }
    }
}
impl ::std::convert::From<u64> for NodeId {
    fn from(value: u64) -> Self {
        Self::Integer(value)
    }
}
///`NodeRole`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "anyOf": [
///    {
///      "type": "string",
///      "const": "Cyber"
///    },
///    {
///      "type": "string",
///      "const": "Physics"
///    }
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum NodeRole {
    Cyber,
    Physics,
}
impl ::std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Cyber => f.write_str("Cyber"),
            Self::Physics => f.write_str("Physics"),
        }
    }
}
impl ::std::str::FromStr for NodeRole {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "Cyber" => Ok(Self::Cyber),
            "Physics" => Ok(Self::Physics),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for NodeRole {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for NodeRole {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for NodeRole {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///Supported inter-node communication protocols.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Protocol.json",
///  "description": "Supported inter-node communication protocols.",
///  "type": "string",
///  "pattern": "^(Ethernet|Uart|CanFd|Spi|FlexRay|Lin|Rf802154|RfHci|eth|uart|canfd|spi|flexray|lin|rf802154|rfhci|ethernet)$",
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[serde(transparent)]
pub struct Protocol(::std::string::String);
impl ::std::ops::Deref for Protocol {
    type Target = ::std::string::String;
    fn deref(&self) -> &::std::string::String {
        &self.0
    }
}
impl ::std::convert::From<Protocol> for ::std::string::String {
    fn from(value: Protocol) -> Self {
        value.0
    }
}
impl ::std::str::FromStr for Protocol {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        static PATTERN: ::std::sync::LazyLock<::regress::Regex> = ::std::sync::LazyLock::new(
            || {
                ::regress::Regex::new(
                    "^(Ethernet|Uart|CanFd|Spi|FlexRay|Lin|Rf802154|RfHci|eth|uart|canfd|spi|flexray|lin|rf802154|rfhci|ethernet)$",
                )
                .unwrap()
            },
        );
        if PATTERN.find(value).is_none() {
            return Err(
                "doesn't match pattern \"^(Ethernet|Uart|CanFd|Spi|FlexRay|Lin|Rf802154|RfHci|eth|uart|canfd|spi|flexray|lin|rf802154|rfhci|ethernet)$\""
                    .into(),
            );
        }
        Ok(Self(value.to_string()))
    }
}
impl ::std::convert::TryFrom<&str> for Protocol {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for Protocol {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for Protocol {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl<'de> ::serde::Deserialize<'de> for Protocol {
    fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
    where
        D: ::serde::Deserializer<'de>,
    {
        ::std::string::String::deserialize(deserializer)?
            .parse()
            .map_err(|e: self::error::ConversionError| {
                <D::Error as ::serde::de::Error>::custom(e.to_string())
            })
    }
}
///`RecordUnknown`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "RecordUnknown.json",
///  "type": "object",
///  "$schema": "https://json-schema.org/draft/2020-12/schema",
///  "unevaluatedProperties": {}
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
#[serde(transparent)]
pub struct RecordUnknown(pub ::serde_json::Map<::std::string::String, ::serde_json::Value>);
impl ::std::ops::Deref for RecordUnknown {
    type Target = ::serde_json::Map<::std::string::String, ::serde_json::Value>;
    fn deref(&self) -> &::serde_json::Map<::std::string::String, ::serde_json::Value> {
        &self.0
    }
}
impl ::std::convert::From<RecordUnknown>
    for ::serde_json::Map<::std::string::String, ::serde_json::Value>
{
    fn from(value: RecordUnknown) -> Self {
        value.0
    }
}
impl ::std::convert::From<::serde_json::Map<::std::string::String, ::serde_json::Value>>
    for RecordUnknown
{
    fn from(value: ::serde_json::Map<::std::string::String, ::serde_json::Value>) -> Self {
        Self(value)
    }
}
///`Resource`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Resource.json",
///  "type": "object",
///  "required": [
///    "name"
///  ],
///  "properties": {
///    "address": {
///      "$ref": "#/$defs/Address"
///    },
///    "container": {
///      "type": "string"
///    },
///    "interrupts": {
///      "type": "array",
///      "items": {
///        "oneOf": [
///          {
///            "type": "string"
///          },
///          {
///            "type": "integer"
///          }
///        ]
///      }
///    },
///    "name": {
///      "$ref": "#/$defs/NodeID"
///    },
///    "parent": {
///      "type": "string"
///    },
///    "properties": {
///      "$ref": "#/$defs/RecordUnknown"
///    },
///    "renode_type": {
///      "type": "string"
///    },
///    "size": {
///      "$ref": "#/$defs/Address"
///    },
///    "type": {
///      "type": "string"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Resource {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub address: ::std::option::Option<Address>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub container: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub interrupts: ::std::vec::Vec<ResourceInterruptsItem>,
    pub name: NodeId,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub parent: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub properties: ::std::option::Option<RecordUnknown>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub renode_type: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub size: ::std::option::Option<Address>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "::std::option::Option::is_none"
    )]
    pub type_: ::std::option::Option<::std::string::String>,
}
impl Resource {
    pub fn builder() -> builder::Resource {
        Default::default()
    }
}
///`ResourceInterruptsItem`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "oneOf": [
///    {
///      "type": "string"
///    },
///    {
///      "type": "integer"
///    }
///  ]
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum ResourceInterruptsItem {
    String(::std::string::String),
    Integer(i64),
}
impl ::std::fmt::Display for ResourceInterruptsItem {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match self {
            Self::String(x) => x.fmt(f),
            Self::Integer(x) => x.fmt(f),
        }
    }
}
impl ::std::convert::From<i64> for ResourceInterruptsItem {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}
///`Topology`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "Topology.json",
///  "type": "object",
///  "properties": {
///    "global_seed": {
///      "type": "string"
///    },
///    "links": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/WireLink"
///      }
///    },
///    "max_messages_per_node_per_quantum": {
///      "type": "integer",
///      "maximum": 4294967295.0,
///      "minimum": 0.0
///    },
///    "nodes": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Node"
///      }
///    },
///    "transport": {
///      "anyOf": [
///        {
///          "type": "string",
///          "const": "zenoh"
///        },
///        {
///          "type": "string",
///          "const": "unix"
///        }
///      ]
///    },
///    "wireless": {
///      "$ref": "#/$defs/WirelessMedium"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct Topology {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub global_seed: ::std::option::Option<::std::string::String>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub links: ::std::vec::Vec<WireLink>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub max_messages_per_node_per_quantum: ::std::option::Option<u32>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub nodes: ::std::vec::Vec<Node>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub transport: ::std::option::Option<TopologyTransport>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub wireless: ::std::option::Option<WirelessMedium>,
}
impl ::std::default::Default for Topology {
    fn default() -> Self {
        Self {
            global_seed: Default::default(),
            links: Default::default(),
            max_messages_per_node_per_quantum: Default::default(),
            nodes: Default::default(),
            transport: Default::default(),
            wireless: Default::default(),
        }
    }
}
impl Topology {
    pub fn builder() -> builder::Topology {
        Default::default()
    }
}
///`TopologyTransport`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "anyOf": [
///    {
///      "type": "string",
///      "const": "zenoh"
///    },
///    {
///      "type": "string",
///      "const": "unix"
///    }
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum TopologyTransport {
    #[serde(rename = "zenoh")]
    Zenoh,
    #[serde(rename = "unix")]
    Unix,
}
impl ::std::fmt::Display for TopologyTransport {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Zenoh => f.write_str("zenoh"),
            Self::Unix => f.write_str("unix"),
        }
    }
}
impl ::std::str::FromStr for TopologyTransport {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "zenoh" => Ok(Self::Zenoh),
            "unix" => Ok(Self::Unix),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for TopologyTransport {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for TopologyTransport {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for TopologyTransport {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///`WireLink`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "WireLink.json",
///  "type": "object",
///  "required": [
///    "nodes",
///    "type"
///  ],
///  "properties": {
///    "baud": {
///      "type": "integer",
///      "maximum": 4294967295.0,
///      "minimum": 0.0
///    },
///    "nodes": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/NodeID"
///      }
///    },
///    "type": {
///      "$ref": "#/$defs/Protocol",
///      "alias": "type"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct WireLink {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub baud: ::std::option::Option<u32>,
    pub nodes: ::std::vec::Vec<NodeId>,
    #[serde(rename = "type")]
    pub type_: Protocol,
}
impl WireLink {
    pub fn builder() -> builder::WireLink {
        Default::default()
    }
}
///`WirelessMedium`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "WirelessMedium.json",
///  "type": "object",
///  "required": [
///    "max_range_m",
///    "medium",
///    "nodes"
///  ],
///  "properties": {
///    "max_range_m": {
///      "type": "number"
///    },
///    "medium": {
///      "type": "string"
///    },
///    "nodes": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/WirelessNode"
///      }
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct WirelessMedium {
    pub max_range_m: f64,
    pub medium: ::std::string::String,
    pub nodes: ::std::vec::Vec<WirelessNode>,
}
impl WirelessMedium {
    pub fn builder() -> builder::WirelessMedium {
        Default::default()
    }
}
///`WirelessNode`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "WirelessNode.json",
///  "type": "object",
///  "required": [
///    "initial_position",
///    "name"
///  ],
///  "properties": {
///    "initial_position": {
///      "$ref": "#/$defs/Coordinate"
///    },
///    "name": {
///      "$ref": "#/$defs/NodeID"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct WirelessNode {
    pub initial_position: Coordinate,
    pub name: NodeId,
}
impl WirelessNode {
    pub fn builder() -> builder::WirelessNode {
        Default::default()
    }
}
///`World`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "World.json",
///  "type": "object",
///  "properties": {
///    "machine": {
///      "$ref": "#/$defs/Machine"
///    },
///    "memory": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Resource"
///      }
///    },
///    "nodes": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Node"
///      }
///    },
///    "peripherals": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Resource"
///      }
///    },
///    "topology": {
///      "$ref": "#/$defs/Topology"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct World {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub machine: ::std::option::Option<Machine>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub memory: ::std::vec::Vec<Resource>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub nodes: ::std::vec::Vec<Node>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub peripherals: ::std::vec::Vec<Resource>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub topology: ::std::option::Option<Topology>,
}
impl ::std::default::Default for World {
    fn default() -> Self {
        Self {
            machine: Default::default(),
            memory: Default::default(),
            nodes: Default::default(),
            peripherals: Default::default(),
            topology: Default::default(),
        }
    }
}
impl World {
    pub fn builder() -> builder::World {
        Default::default()
    }
}
///`WorldSchema`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "$id": "WorldSchema.json",
///  "type": "object",
///  "properties": {
///    "machine": {
///      "$ref": "#/$defs/Machine"
///    },
///    "memory": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Resource"
///      }
///    },
///    "nodes": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Node"
///      }
///    },
///    "peripherals": {
///      "type": "array",
///      "items": {
///        "$ref": "#/$defs/Resource"
///      }
///    },
///    "topology": {
///      "$ref": "#/$defs/Topology"
///    }
///  },
///  "$schema": "https://json-schema.org/draft/2020-12/schema"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug)]
pub struct WorldSchema {
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub machine: ::std::option::Option<Machine>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub memory: ::std::vec::Vec<Resource>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub nodes: ::std::vec::Vec<Node>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub peripherals: ::std::vec::Vec<Resource>,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub topology: ::std::option::Option<Topology>,
}
impl ::std::default::Default for WorldSchema {
    fn default() -> Self {
        Self {
            machine: Default::default(),
            memory: Default::default(),
            nodes: Default::default(),
            peripherals: Default::default(),
            topology: Default::default(),
        }
    }
}
impl WorldSchema {
    pub fn builder() -> builder::WorldSchema {
        Default::default()
    }
}
/// Types for composing complex structures.
pub mod builder {
    #[derive(Clone, Debug)]
    pub struct Coordinate {
        x: ::std::result::Result<f64, ::std::string::String>,
        y: ::std::result::Result<f64, ::std::string::String>,
        z: ::std::result::Result<f64, ::std::string::String>,
    }
    impl ::std::default::Default for Coordinate {
        fn default() -> Self {
            Self {
                x: Err("no value supplied for x".to_string()),
                y: Err("no value supplied for y".to_string()),
                z: Err("no value supplied for z".to_string()),
            }
        }
    }
    impl Coordinate {
        pub fn x<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<f64>,
            T::Error: ::std::fmt::Display,
        {
            self.x = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for x: {e}"));
            self
        }
        pub fn y<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<f64>,
            T::Error: ::std::fmt::Display,
        {
            self.y = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for y: {e}"));
            self
        }
        pub fn z<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<f64>,
            T::Error: ::std::fmt::Display,
        {
            self.z = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for z: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Coordinate> for super::Coordinate {
        type Error = super::error::ConversionError;
        fn try_from(
            value: Coordinate,
        ) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                x: value.x?,
                y: value.y?,
                z: value.z?,
            })
        }
    }
    impl ::std::convert::From<super::Coordinate> for Coordinate {
        fn from(value: super::Coordinate) -> Self {
            Self {
                x: Ok(value.x),
                y: Ok(value.y),
                z: Ok(value.z),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct Cpu {
        isa: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        memory: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        mmu_type: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        name: ::std::result::Result<::std::string::String, ::std::string::String>,
        type_: ::std::result::Result<::std::string::String, ::std::string::String>,
    }
    impl ::std::default::Default for Cpu {
        fn default() -> Self {
            Self {
                isa: Ok(Default::default()),
                memory: Ok(Default::default()),
                mmu_type: Ok(Default::default()),
                name: Err("no value supplied for name".to_string()),
                type_: Err("no value supplied for type_".to_string()),
            }
        }
    }
    impl Cpu {
        pub fn isa<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.isa = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for isa: {e}"));
            self
        }
        pub fn memory<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.memory = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for memory: {e}"));
            self
        }
        pub fn mmu_type<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.mmu_type = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for mmu_type: {e}"));
            self
        }
        pub fn name<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::string::String>,
            T::Error: ::std::fmt::Display,
        {
            self.name = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for name: {e}"));
            self
        }
        pub fn type_<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::string::String>,
            T::Error: ::std::fmt::Display,
        {
            self.type_ = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for type_: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Cpu> for super::Cpu {
        type Error = super::error::ConversionError;
        fn try_from(value: Cpu) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                isa: value.isa?,
                memory: value.memory?,
                mmu_type: value.mmu_type?,
                name: value.name?,
                type_: value.type_?,
            })
        }
    }
    impl ::std::convert::From<super::Cpu> for Cpu {
        fn from(value: super::Cpu) -> Self {
            Self {
                isa: Ok(value.isa),
                memory: Ok(value.memory),
                mmu_type: Ok(value.mmu_type),
                name: Ok(value.name),
                type_: Ok(value.type_),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct Machine {
        cpus: ::std::result::Result<::std::vec::Vec<super::Cpu>, ::std::string::String>,
        name: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        type_: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
    }
    impl ::std::default::Default for Machine {
        fn default() -> Self {
            Self {
                cpus: Ok(Default::default()),
                name: Ok(Default::default()),
                type_: Ok(Default::default()),
            }
        }
    }
    impl Machine {
        pub fn cpus<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Cpu>>,
            T::Error: ::std::fmt::Display,
        {
            self.cpus = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for cpus: {e}"));
            self
        }
        pub fn name<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.name = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for name: {e}"));
            self
        }
        pub fn type_<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.type_ = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for type_: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Machine> for super::Machine {
        type Error = super::error::ConversionError;
        fn try_from(value: Machine) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                cpus: value.cpus?,
                name: value.name?,
                type_: value.type_?,
            })
        }
    }
    impl ::std::convert::From<super::Machine> for Machine {
        fn from(value: super::Machine) -> Self {
            Self {
                cpus: Ok(value.cpus),
                name: Ok(value.name),
                type_: Ok(value.type_),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct Node {
        name: ::std::result::Result<super::NodeId, ::std::string::String>,
        role: ::std::result::Result<::std::option::Option<super::NodeRole>, ::std::string::String>,
    }
    impl ::std::default::Default for Node {
        fn default() -> Self {
            Self {
                name: Err("no value supplied for name".to_string()),
                role: Ok(Default::default()),
            }
        }
    }
    impl Node {
        pub fn name<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<super::NodeId>,
            T::Error: ::std::fmt::Display,
        {
            self.name = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for name: {e}"));
            self
        }
        pub fn role<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::NodeRole>>,
            T::Error: ::std::fmt::Display,
        {
            self.role = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for role: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Node> for super::Node {
        type Error = super::error::ConversionError;
        fn try_from(value: Node) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                name: value.name?,
                role: value.role?,
            })
        }
    }
    impl ::std::convert::From<super::Node> for Node {
        fn from(value: super::Node) -> Self {
            Self {
                name: Ok(value.name),
                role: Ok(value.role),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct Resource {
        address:
            ::std::result::Result<::std::option::Option<super::Address>, ::std::string::String>,
        container: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        interrupts: ::std::result::Result<
            ::std::vec::Vec<super::ResourceInterruptsItem>,
            ::std::string::String,
        >,
        name: ::std::result::Result<super::NodeId, ::std::string::String>,
        parent: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        properties: ::std::result::Result<
            ::std::option::Option<super::RecordUnknown>,
            ::std::string::String,
        >,
        renode_type: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        size: ::std::result::Result<::std::option::Option<super::Address>, ::std::string::String>,
        type_: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
    }
    impl ::std::default::Default for Resource {
        fn default() -> Self {
            Self {
                address: Ok(Default::default()),
                container: Ok(Default::default()),
                interrupts: Ok(Default::default()),
                name: Err("no value supplied for name".to_string()),
                parent: Ok(Default::default()),
                properties: Ok(Default::default()),
                renode_type: Ok(Default::default()),
                size: Ok(Default::default()),
                type_: Ok(Default::default()),
            }
        }
    }
    impl Resource {
        pub fn address<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Address>>,
            T::Error: ::std::fmt::Display,
        {
            self.address = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for address: {e}"));
            self
        }
        pub fn container<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.container = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for container: {e}"));
            self
        }
        pub fn interrupts<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::ResourceInterruptsItem>>,
            T::Error: ::std::fmt::Display,
        {
            self.interrupts = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for interrupts: {e}"));
            self
        }
        pub fn name<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<super::NodeId>,
            T::Error: ::std::fmt::Display,
        {
            self.name = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for name: {e}"));
            self
        }
        pub fn parent<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.parent = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for parent: {e}"));
            self
        }
        pub fn properties<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::RecordUnknown>>,
            T::Error: ::std::fmt::Display,
        {
            self.properties = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for properties: {e}"));
            self
        }
        pub fn renode_type<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.renode_type = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for renode_type: {e}"));
            self
        }
        pub fn size<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Address>>,
            T::Error: ::std::fmt::Display,
        {
            self.size = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for size: {e}"));
            self
        }
        pub fn type_<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.type_ = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for type_: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Resource> for super::Resource {
        type Error = super::error::ConversionError;
        fn try_from(value: Resource) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                address: value.address?,
                container: value.container?,
                interrupts: value.interrupts?,
                name: value.name?,
                parent: value.parent?,
                properties: value.properties?,
                renode_type: value.renode_type?,
                size: value.size?,
                type_: value.type_?,
            })
        }
    }
    impl ::std::convert::From<super::Resource> for Resource {
        fn from(value: super::Resource) -> Self {
            Self {
                address: Ok(value.address),
                container: Ok(value.container),
                interrupts: Ok(value.interrupts),
                name: Ok(value.name),
                parent: Ok(value.parent),
                properties: Ok(value.properties),
                renode_type: Ok(value.renode_type),
                size: Ok(value.size),
                type_: Ok(value.type_),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct Topology {
        global_seed: ::std::result::Result<
            ::std::option::Option<::std::string::String>,
            ::std::string::String,
        >,
        links: ::std::result::Result<::std::vec::Vec<super::WireLink>, ::std::string::String>,
        max_messages_per_node_per_quantum:
            ::std::result::Result<::std::option::Option<u32>, ::std::string::String>,
        nodes: ::std::result::Result<::std::vec::Vec<super::Node>, ::std::string::String>,
        transport: ::std::result::Result<
            ::std::option::Option<super::TopologyTransport>,
            ::std::string::String,
        >,
        wireless: ::std::result::Result<
            ::std::option::Option<super::WirelessMedium>,
            ::std::string::String,
        >,
    }
    impl ::std::default::Default for Topology {
        fn default() -> Self {
            Self {
                global_seed: Ok(Default::default()),
                links: Ok(Default::default()),
                max_messages_per_node_per_quantum: Ok(Default::default()),
                nodes: Ok(Default::default()),
                transport: Ok(Default::default()),
                wireless: Ok(Default::default()),
            }
        }
    }
    impl Topology {
        pub fn global_seed<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<::std::string::String>>,
            T::Error: ::std::fmt::Display,
        {
            self.global_seed = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for global_seed: {e}"));
            self
        }
        pub fn links<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::WireLink>>,
            T::Error: ::std::fmt::Display,
        {
            self.links = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for links: {e}"));
            self
        }
        pub fn max_messages_per_node_per_quantum<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<u32>>,
            T::Error: ::std::fmt::Display,
        {
            self.max_messages_per_node_per_quantum = value.try_into().map_err(|e| {
                format!(
                    "error converting supplied value for max_messages_per_node_per_quantum: {e}"
                )
            });
            self
        }
        pub fn nodes<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Node>>,
            T::Error: ::std::fmt::Display,
        {
            self.nodes = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for nodes: {e}"));
            self
        }
        pub fn transport<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::TopologyTransport>>,
            T::Error: ::std::fmt::Display,
        {
            self.transport = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for transport: {e}"));
            self
        }
        pub fn wireless<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::WirelessMedium>>,
            T::Error: ::std::fmt::Display,
        {
            self.wireless = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for wireless: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<Topology> for super::Topology {
        type Error = super::error::ConversionError;
        fn try_from(value: Topology) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                global_seed: value.global_seed?,
                links: value.links?,
                max_messages_per_node_per_quantum: value.max_messages_per_node_per_quantum?,
                nodes: value.nodes?,
                transport: value.transport?,
                wireless: value.wireless?,
            })
        }
    }
    impl ::std::convert::From<super::Topology> for Topology {
        fn from(value: super::Topology) -> Self {
            Self {
                global_seed: Ok(value.global_seed),
                links: Ok(value.links),
                max_messages_per_node_per_quantum: Ok(value.max_messages_per_node_per_quantum),
                nodes: Ok(value.nodes),
                transport: Ok(value.transport),
                wireless: Ok(value.wireless),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct WireLink {
        baud: ::std::result::Result<::std::option::Option<u32>, ::std::string::String>,
        nodes: ::std::result::Result<::std::vec::Vec<super::NodeId>, ::std::string::String>,
        type_: ::std::result::Result<super::Protocol, ::std::string::String>,
    }
    impl ::std::default::Default for WireLink {
        fn default() -> Self {
            Self {
                baud: Ok(Default::default()),
                nodes: Err("no value supplied for nodes".to_string()),
                type_: Err("no value supplied for type_".to_string()),
            }
        }
    }
    impl WireLink {
        pub fn baud<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<u32>>,
            T::Error: ::std::fmt::Display,
        {
            self.baud = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for baud: {e}"));
            self
        }
        pub fn nodes<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::NodeId>>,
            T::Error: ::std::fmt::Display,
        {
            self.nodes = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for nodes: {e}"));
            self
        }
        pub fn type_<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<super::Protocol>,
            T::Error: ::std::fmt::Display,
        {
            self.type_ = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for type_: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<WireLink> for super::WireLink {
        type Error = super::error::ConversionError;
        fn try_from(value: WireLink) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                baud: value.baud?,
                nodes: value.nodes?,
                type_: value.type_?,
            })
        }
    }
    impl ::std::convert::From<super::WireLink> for WireLink {
        fn from(value: super::WireLink) -> Self {
            Self {
                baud: Ok(value.baud),
                nodes: Ok(value.nodes),
                type_: Ok(value.type_),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct WirelessMedium {
        max_range_m: ::std::result::Result<f64, ::std::string::String>,
        medium: ::std::result::Result<::std::string::String, ::std::string::String>,
        nodes: ::std::result::Result<::std::vec::Vec<super::WirelessNode>, ::std::string::String>,
    }
    impl ::std::default::Default for WirelessMedium {
        fn default() -> Self {
            Self {
                max_range_m: Err("no value supplied for max_range_m".to_string()),
                medium: Err("no value supplied for medium".to_string()),
                nodes: Err("no value supplied for nodes".to_string()),
            }
        }
    }
    impl WirelessMedium {
        pub fn max_range_m<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<f64>,
            T::Error: ::std::fmt::Display,
        {
            self.max_range_m = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for max_range_m: {e}"));
            self
        }
        pub fn medium<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::string::String>,
            T::Error: ::std::fmt::Display,
        {
            self.medium = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for medium: {e}"));
            self
        }
        pub fn nodes<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::WirelessNode>>,
            T::Error: ::std::fmt::Display,
        {
            self.nodes = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for nodes: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<WirelessMedium> for super::WirelessMedium {
        type Error = super::error::ConversionError;
        fn try_from(
            value: WirelessMedium,
        ) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                max_range_m: value.max_range_m?,
                medium: value.medium?,
                nodes: value.nodes?,
            })
        }
    }
    impl ::std::convert::From<super::WirelessMedium> for WirelessMedium {
        fn from(value: super::WirelessMedium) -> Self {
            Self {
                max_range_m: Ok(value.max_range_m),
                medium: Ok(value.medium),
                nodes: Ok(value.nodes),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct WirelessNode {
        initial_position: ::std::result::Result<super::Coordinate, ::std::string::String>,
        name: ::std::result::Result<super::NodeId, ::std::string::String>,
    }
    impl ::std::default::Default for WirelessNode {
        fn default() -> Self {
            Self {
                initial_position: Err("no value supplied for initial_position".to_string()),
                name: Err("no value supplied for name".to_string()),
            }
        }
    }
    impl WirelessNode {
        pub fn initial_position<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<super::Coordinate>,
            T::Error: ::std::fmt::Display,
        {
            self.initial_position = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for initial_position: {e}"));
            self
        }
        pub fn name<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<super::NodeId>,
            T::Error: ::std::fmt::Display,
        {
            self.name = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for name: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<WirelessNode> for super::WirelessNode {
        type Error = super::error::ConversionError;
        fn try_from(
            value: WirelessNode,
        ) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                initial_position: value.initial_position?,
                name: value.name?,
            })
        }
    }
    impl ::std::convert::From<super::WirelessNode> for WirelessNode {
        fn from(value: super::WirelessNode) -> Self {
            Self {
                initial_position: Ok(value.initial_position),
                name: Ok(value.name),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct World {
        machine:
            ::std::result::Result<::std::option::Option<super::Machine>, ::std::string::String>,
        memory: ::std::result::Result<::std::vec::Vec<super::Resource>, ::std::string::String>,
        nodes: ::std::result::Result<::std::vec::Vec<super::Node>, ::std::string::String>,
        peripherals: ::std::result::Result<::std::vec::Vec<super::Resource>, ::std::string::String>,
        topology:
            ::std::result::Result<::std::option::Option<super::Topology>, ::std::string::String>,
    }
    impl ::std::default::Default for World {
        fn default() -> Self {
            Self {
                machine: Ok(Default::default()),
                memory: Ok(Default::default()),
                nodes: Ok(Default::default()),
                peripherals: Ok(Default::default()),
                topology: Ok(Default::default()),
            }
        }
    }
    impl World {
        pub fn machine<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Machine>>,
            T::Error: ::std::fmt::Display,
        {
            self.machine = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for machine: {e}"));
            self
        }
        pub fn memory<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Resource>>,
            T::Error: ::std::fmt::Display,
        {
            self.memory = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for memory: {e}"));
            self
        }
        pub fn nodes<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Node>>,
            T::Error: ::std::fmt::Display,
        {
            self.nodes = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for nodes: {e}"));
            self
        }
        pub fn peripherals<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Resource>>,
            T::Error: ::std::fmt::Display,
        {
            self.peripherals = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for peripherals: {e}"));
            self
        }
        pub fn topology<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Topology>>,
            T::Error: ::std::fmt::Display,
        {
            self.topology = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for topology: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<World> for super::World {
        type Error = super::error::ConversionError;
        fn try_from(value: World) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                machine: value.machine?,
                memory: value.memory?,
                nodes: value.nodes?,
                peripherals: value.peripherals?,
                topology: value.topology?,
            })
        }
    }
    impl ::std::convert::From<super::World> for World {
        fn from(value: super::World) -> Self {
            Self {
                machine: Ok(value.machine),
                memory: Ok(value.memory),
                nodes: Ok(value.nodes),
                peripherals: Ok(value.peripherals),
                topology: Ok(value.topology),
            }
        }
    }
    #[derive(Clone, Debug)]
    pub struct WorldSchema {
        machine:
            ::std::result::Result<::std::option::Option<super::Machine>, ::std::string::String>,
        memory: ::std::result::Result<::std::vec::Vec<super::Resource>, ::std::string::String>,
        nodes: ::std::result::Result<::std::vec::Vec<super::Node>, ::std::string::String>,
        peripherals: ::std::result::Result<::std::vec::Vec<super::Resource>, ::std::string::String>,
        topology:
            ::std::result::Result<::std::option::Option<super::Topology>, ::std::string::String>,
    }
    impl ::std::default::Default for WorldSchema {
        fn default() -> Self {
            Self {
                machine: Ok(Default::default()),
                memory: Ok(Default::default()),
                nodes: Ok(Default::default()),
                peripherals: Ok(Default::default()),
                topology: Ok(Default::default()),
            }
        }
    }
    impl WorldSchema {
        pub fn machine<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Machine>>,
            T::Error: ::std::fmt::Display,
        {
            self.machine = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for machine: {e}"));
            self
        }
        pub fn memory<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Resource>>,
            T::Error: ::std::fmt::Display,
        {
            self.memory = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for memory: {e}"));
            self
        }
        pub fn nodes<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Node>>,
            T::Error: ::std::fmt::Display,
        {
            self.nodes = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for nodes: {e}"));
            self
        }
        pub fn peripherals<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::vec::Vec<super::Resource>>,
            T::Error: ::std::fmt::Display,
        {
            self.peripherals = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for peripherals: {e}"));
            self
        }
        pub fn topology<T>(mut self, value: T) -> Self
        where
            T: ::std::convert::TryInto<::std::option::Option<super::Topology>>,
            T::Error: ::std::fmt::Display,
        {
            self.topology = value
                .try_into()
                .map_err(|e| format!("error converting supplied value for topology: {e}"));
            self
        }
    }
    impl ::std::convert::TryFrom<WorldSchema> for super::WorldSchema {
        type Error = super::error::ConversionError;
        fn try_from(
            value: WorldSchema,
        ) -> ::std::result::Result<Self, super::error::ConversionError> {
            Ok(Self {
                machine: value.machine?,
                memory: value.memory?,
                nodes: value.nodes?,
                peripherals: value.peripherals?,
                topology: value.topology?,
            })
        }
    }
    impl ::std::convert::From<super::WorldSchema> for WorldSchema {
        fn from(value: super::WorldSchema) -> Self {
            Self {
                machine: Ok(value.machine),
                memory: Ok(value.memory),
                nodes: Ok(value.nodes),
                peripherals: Ok(value.peripherals),
                topology: Ok(value.topology),
            }
        }
    }
}
