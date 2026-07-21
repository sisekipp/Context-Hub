use std::{sync::Arc, time::Duration};

use context_hub_api::context_hub::v1::{
    ExternalFunctionRequest, external_function_service_client::ExternalFunctionServiceClient,
};
use context_hub_domain::{
    FunctionDefinition, FunctionImplementation, OntologyDefinition, ScalarType, TypeReference,
};
use context_hub_storage::SourceObjectStore;
use serde_json::{Map, Number, Value};
use tonic::Request;
use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(5);
const WASM_FUEL: u64 = 1_000_000;
const WASM_MEMORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_FUNCTION_JSON_BYTES: usize = 1024 * 1024;

pub async fn execute(
    definition: &OntologyDefinition,
    function: &FunctionDefinition,
    arguments: Value,
    source_store: Arc<dyn SourceObjectStore>,
) -> Result<(Value, &'static str), String> {
    let arguments = validate_arguments(definition, function, arguments)?;
    let result = match &function.implementation {
        FunctionImplementation::Expression { expression } => {
            (evaluate_expression(expression, &arguments)?, "expression")
        }
        FunctionImplementation::ExternalGrpc { endpoint, method } => (
            execute_external(endpoint, method, function, &arguments).await?,
            "external_grpc",
        ),
        FunctionImplementation::Wasm {
            artifact_uri,
            entrypoint,
        } => {
            let bytes = load_wasm(source_store, artifact_uri).await?;
            let arguments = serde_json::to_vec(&arguments).map_err(|error| error.to_string())?;
            let entrypoint = entrypoint.clone();
            let result =
                tokio::task::spawn_blocking(move || execute_wasm(&bytes, &entrypoint, &arguments))
                    .await
                    .map_err(|error| format!("WASM worker failed: {error}"))??;
            (result, "wasm")
        }
    };
    validate_value(definition, &function.output, &result.0, "result")?;
    Ok(result)
}

fn validate_arguments(
    definition: &OntologyDefinition,
    function: &FunctionDefinition,
    arguments: Value,
) -> Result<Map<String, Value>, String> {
    let Value::Object(arguments) = arguments else {
        return Err("function arguments must be a JSON object".into());
    };
    for name in arguments.keys() {
        if !function.inputs.iter().any(|input| input.api_name == *name) {
            return Err(format!("unknown function argument '{name}'"));
        }
    }
    for input in &function.inputs {
        match arguments.get(&input.api_name) {
            Some(Value::Null) | None if input.required => {
                return Err(format!(
                    "required function argument '{}' is missing",
                    input.api_name
                ));
            }
            Some(value) if !value.is_null() => validate_value(
                definition,
                &input.value_type,
                value,
                &format!("arguments.{}", input.api_name),
            )?,
            _ => {}
        }
    }
    Ok(arguments)
}

fn validate_value(
    definition: &OntologyDefinition,
    reference: &TypeReference,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    match reference {
        TypeReference::Scalar { value_type } => {
            if value_type.list {
                let values = value
                    .as_array()
                    .ok_or_else(|| format!("{path} must be a list"))?;
                for (index, item) in values.iter().enumerate() {
                    validate_scalar(&value_type.scalar, item, &format!("{path}[{index}]"))?;
                }
            } else {
                validate_scalar(&value_type.scalar, value, path)?;
            }
        }
        TypeReference::ValueType { api_name, list } => {
            let value_type = definition
                .value_types
                .iter()
                .find(|candidate| candidate.api_name == *api_name)
                .ok_or_else(|| format!("unknown value type '{api_name}'"))?;
            if *list {
                let values = value
                    .as_array()
                    .ok_or_else(|| format!("{path} must be a list"))?;
                for (index, item) in values.iter().enumerate() {
                    validate_scalar(&value_type.base_type, item, &format!("{path}[{index}]"))?;
                }
            } else {
                validate_scalar(&value_type.base_type, value, path)?;
            }
        }
        TypeReference::Struct { api_name, list } => {
            let struct_type = definition
                .struct_types
                .iter()
                .find(|candidate| candidate.api_name == *api_name)
                .ok_or_else(|| format!("unknown struct type '{api_name}'"))?;
            let values: Vec<&Value> = if *list {
                value
                    .as_array()
                    .ok_or_else(|| format!("{path} must be a list"))?
                    .iter()
                    .collect()
            } else {
                vec![value]
            };
            for (index, item) in values.into_iter().enumerate() {
                let item_path = if *list {
                    format!("{path}[{index}]")
                } else {
                    path.to_owned()
                };
                let object = item
                    .as_object()
                    .ok_or_else(|| format!("{item_path} must be an object"))?;
                for field in &struct_type.fields {
                    match object.get(&field.api_name) {
                        Some(Value::Null) | None if field.required => {
                            return Err(format!("{item_path}.{} is required", field.api_name));
                        }
                        Some(value) if !value.is_null() => validate_value(
                            definition,
                            &field.value_type,
                            value,
                            &format!("{item_path}.{}", field.api_name),
                        )?,
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_scalar(scalar: &ScalarType, value: &Value, path: &str) -> Result<(), String> {
    let valid = match scalar {
        ScalarType::String | ScalarType::Date | ScalarType::Timestamp | ScalarType::Uuid => {
            value.is_string()
        }
        ScalarType::Boolean => value.is_boolean(),
        ScalarType::Int64 => value.as_i64().is_some() || value.as_u64().is_some(),
        ScalarType::Float64 | ScalarType::Decimal => value.is_number(),
        ScalarType::Enum { values } => value
            .as_str()
            .is_some_and(|value| values.iter().any(|item| item == value)),
        ScalarType::Json => true,
    };
    valid
        .then_some(())
        .ok_or_else(|| format!("{path} has the wrong type"))
}

async fn execute_external(
    endpoint: &str,
    method: &str,
    function: &FunctionDefinition,
    arguments: &Map<String, Value>,
) -> Result<Value, String> {
    if method != "invoke" && method != "/context_hub.v1.ExternalFunctionService/Invoke" {
        return Err("external gRPC method must use the ContextHub Invoke contract".into());
    }
    validate_external_endpoint(endpoint)?;
    let mut client = tokio::time::timeout(
        EXTERNAL_TIMEOUT,
        ExternalFunctionServiceClient::connect(endpoint.to_owned()),
    )
    .await
    .map_err(|_| "external gRPC connection timed out".to_owned())?
    .map_err(|error| format!("external gRPC connection failed: {error}"))?;
    let request = ExternalFunctionRequest {
        function_api_name: function.api_name.clone(),
        arguments_json: Value::Object(arguments.clone()).to_string(),
    };
    let response = tokio::time::timeout(EXTERNAL_TIMEOUT, client.invoke(Request::new(request)))
        .await
        .map_err(|_| "external gRPC function timed out".to_owned())?
        .map_err(|error| format!("external gRPC function failed: {error}"))?
        .into_inner();
    if response.result_json.len() > MAX_FUNCTION_JSON_BYTES {
        return Err("external gRPC result exceeds 1 MiB".into());
    }
    serde_json::from_str(&response.result_json)
        .map_err(|error| format!("external gRPC result is not valid JSON: {error}"))
}

fn validate_external_endpoint(endpoint: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| "external gRPC endpoint is invalid")?;
    let origin = parsed.origin().ascii_serialization();
    let configured = std::env::var("FUNCTION_GRPC_ALLOWLIST").unwrap_or_default();
    let allowed = configured
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .any(|item| item == origin);
    let dev_local = std::env::var("AUTH_MODE").unwrap_or_default() == "dev"
        && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if !allowed && !dev_local {
        return Err(format!(
            "external gRPC origin '{origin}' is not allowlisted"
        ));
    }
    if parsed.scheme() != "https" && !dev_local {
        return Err("external gRPC endpoints must use HTTPS".into());
    }
    Ok(())
}

async fn load_wasm(
    source_store: Arc<dyn SourceObjectStore>,
    artifact_uri: &str,
) -> Result<Vec<u8>, String> {
    let key = artifact_uri
        .strip_prefix("object://")
        .or_else(|| artifact_uri.strip_prefix("s3://context-hub/"))
        .ok_or_else(|| "WASM artifact URI must use object:// or s3://context-hub/".to_owned())?;
    if key.is_empty() || key.contains("..") {
        return Err("WASM artifact key is invalid".into());
    }
    source_store
        .get(key)
        .await
        .map_err(|error| format!("WASM artifact could not be loaded: {error}"))
}

struct WasmState {
    limits: StoreLimits,
}

fn execute_wasm(bytes: &[u8], entrypoint: &str, arguments: &[u8]) -> Result<Value, String> {
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module =
        Module::new(&engine, bytes).map_err(|error| format!("invalid WASM module: {error}"))?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(WASM_MEMORY_BYTES)
        .build();
    let mut store = Store::new(&engine, WasmState { limits });
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(WASM_FUEL)
        .map_err(|error| error.to_string())?;
    let linker = Linker::new(&engine);
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| {
            format!("WASM module cannot be instantiated without host imports: {error}")
        })?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| "WASM module must export memory".to_owned())?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "alloc")
        .map_err(|_| "WASM module must export alloc(i32) -> i32".to_owned())?;
    let function = instance
        .get_typed_func::<(i32, i32), i64>(&store, entrypoint)
        .map_err(|_| {
            format!("WASM entrypoint '{entrypoint}' must have signature (i32, i32) -> i64")
        })?;
    let input_len = i32::try_from(arguments.len()).map_err(|_| "WASM input is too large")?;
    let input_ptr = alloc
        .call(&mut store, input_len)
        .map_err(|error| format!("WASM allocation failed: {error}"))?;
    memory
        .write(
            &mut store,
            usize::try_from(input_ptr).map_err(|_| "WASM returned an invalid input pointer")?,
            arguments,
        )
        .map_err(|error| format!("WASM input write failed: {error}"))?;
    let packed = function
        .call(&mut store, (input_ptr, input_len))
        .map_err(|error| format!("WASM function trapped: {error}"))?;
    let packed = u64::from_ne_bytes(packed.to_ne_bytes());
    let result_ptr =
        usize::try_from(packed >> 32).map_err(|_| "WASM returned an invalid result pointer")?;
    let result_len = usize::try_from(packed & 0xffff_ffff)
        .map_err(|_| "WASM returned an invalid result length")?;
    if result_len > MAX_FUNCTION_JSON_BYTES {
        return Err("WASM result exceeds 1 MiB".into());
    }
    let mut result = vec![0; result_len];
    memory
        .read(&store, result_ptr, &mut result)
        .map_err(|error| format!("WASM result read failed: {error}"))?;
    serde_json::from_slice(&result)
        .map_err(|error| format!("WASM result is not valid JSON: {error}"))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Identifier(String),
    String(String),
    Number(f64),
    True,
    False,
    Null,
    LeftParen,
    RightParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    NotEq,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    And,
    Or,
    End,
}

struct Lexer<'a> {
    source: &'a [u8],
    position: usize,
}

impl Lexer<'_> {
    fn next(&mut self) -> Result<Token, String> {
        while self
            .source
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
        let Some(&byte) = self.source.get(self.position) else {
            return Ok(Token::End);
        };
        self.position += 1;
        Ok(match byte {
            b'(' => Token::LeftParen,
            b')' => Token::RightParen,
            b',' => Token::Comma,
            b'+' => Token::Plus,
            b'-' => Token::Minus,
            b'*' => Token::Star,
            b'/' => Token::Slash,
            b'=' if self.take(b'=') => Token::Eq,
            b'!' if self.take(b'=') => Token::NotEq,
            b'>' if self.take(b'=') => Token::GreaterEq,
            b'>' => Token::Greater,
            b'<' if self.take(b'=') => Token::LessEq,
            b'<' => Token::Less,
            b'&' if self.take(b'&') => Token::And,
            b'|' if self.take(b'|') => Token::Or,
            b'\'' | b'"' => Token::String(self.string(byte)?),
            digit if digit.is_ascii_digit() => self.number()?,
            first if first.is_ascii_alphabetic() || first == b'_' => self.identifier(),
            _ => {
                return Err(format!(
                    "unexpected character at position {}",
                    self.position
                ));
            }
        })
    }
    fn take(&mut self, expected: u8) -> bool {
        if self.source.get(self.position) == Some(&expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self, quote: u8) -> Result<String, String> {
        let mut value = Vec::new();
        while let Some(&byte) = self.source.get(self.position) {
            self.position += 1;
            if byte == quote {
                return String::from_utf8(value)
                    .map_err(|_| "string literal is not valid UTF-8".into());
            }
            if byte == b'\\' {
                let escaped = *self
                    .source
                    .get(self.position)
                    .ok_or("unterminated string escape")?;
                self.position += 1;
                value.push(match escaped {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'\\' => b'\\',
                    b'\'' => b'\'',
                    b'"' => b'"',
                    _ => return Err("unsupported string escape".into()),
                });
            } else {
                value.push(byte);
            }
        }
        Err("unterminated string".into())
    }
    fn number(&mut self) -> Result<Token, String> {
        let start = self.position - 1;
        while self
            .source
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            self.position += 1;
        }
        let text = std::str::from_utf8(&self.source[start..self.position])
            .map_err(|_| "invalid number")?;
        let value = text.parse().map_err(|_| "invalid number")?;
        Ok(Token::Number(value))
    }
    fn identifier(&mut self) -> Token {
        let start = self.position - 1;
        while self
            .source
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.position += 1;
        }
        match std::str::from_utf8(&self.source[start..self.position]).unwrap_or_default() {
            "true" => Token::True,
            "false" => Token::False,
            "null" => Token::Null,
            value => Token::Identifier(value.to_owned()),
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    arguments: &'a Map<String, Value>,
}

impl<'a> Parser<'a> {
    fn new(expression: &'a str, arguments: &'a Map<String, Value>) -> Result<Self, String> {
        let mut lexer = Lexer {
            source: expression.as_bytes(),
            position: 0,
        };
        let current = lexer.next()?;
        Ok(Self {
            lexer,
            current,
            arguments,
        })
    }
    fn advance(&mut self) -> Result<Token, String> {
        let current = std::mem::replace(&mut self.current, self.lexer.next()?);
        Ok(current)
    }
    fn parse(mut self) -> Result<Value, String> {
        let value = self.or()?;
        if self.current != Token::End {
            return Err("unexpected token after expression".into());
        }
        Ok(value)
    }
    fn or(&mut self) -> Result<Value, String> {
        let mut value = self.and()?;
        while self.current == Token::Or {
            self.advance()?;
            let right = self.and()?;
            value = Value::Bool(as_bool(&value)? || as_bool(&right)?);
        }
        Ok(value)
    }
    fn and(&mut self) -> Result<Value, String> {
        let mut value = self.compare()?;
        while self.current == Token::And {
            self.advance()?;
            let right = self.compare()?;
            value = Value::Bool(as_bool(&value)? && as_bool(&right)?);
        }
        Ok(value)
    }
    fn compare(&mut self) -> Result<Value, String> {
        let mut value = self.term()?;
        loop {
            let operator = self.current.clone();
            if !matches!(
                operator,
                Token::Eq
                    | Token::NotEq
                    | Token::Greater
                    | Token::GreaterEq
                    | Token::Less
                    | Token::LessEq
            ) {
                break;
            }
            self.advance()?;
            let right = self.term()?;
            value = Value::Bool(compare_values(&value, &right, &operator)?);
        }
        Ok(value)
    }
    fn term(&mut self) -> Result<Value, String> {
        let mut value = self.factor()?;
        loop {
            let operator = self.current.clone();
            if !matches!(operator, Token::Plus | Token::Minus) {
                break;
            }
            self.advance()?;
            let right = self.factor()?;
            value = arithmetic(&value, &right, &operator)?;
        }
        Ok(value)
    }
    fn factor(&mut self) -> Result<Value, String> {
        let mut value = self.unary()?;
        loop {
            let operator = self.current.clone();
            if !matches!(operator, Token::Star | Token::Slash) {
                break;
            }
            self.advance()?;
            let right = self.unary()?;
            value = arithmetic(&value, &right, &operator)?;
        }
        Ok(value)
    }
    fn unary(&mut self) -> Result<Value, String> {
        if self.current == Token::Minus {
            self.advance()?;
            return json_number(-as_number(&self.unary()?)?);
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<Value, String> {
        match self.advance()? {
            Token::String(value) => Ok(Value::String(value)),
            Token::Number(value) => json_number(value),
            Token::True => Ok(Value::Bool(true)),
            Token::False => Ok(Value::Bool(false)),
            Token::Null => Ok(Value::Null),
            Token::Identifier(name) if self.current == Token::LeftParen => self.call(&name),
            Token::Identifier(name) => self
                .arguments
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("unknown function argument '{name}'")),
            Token::LeftParen => {
                let value = self.or()?;
                if self.advance()? != Token::RightParen {
                    return Err("missing closing parenthesis".into());
                }
                Ok(value)
            }
            _ => Err("expected a value".into()),
        }
    }
    fn call(&mut self, name: &str) -> Result<Value, String> {
        self.advance()?;
        let mut values = Vec::new();
        if self.current != Token::RightParen {
            loop {
                values.push(self.or()?);
                if self.current != Token::Comma {
                    break;
                }
                self.advance()?;
            }
        }
        if self.advance()? != Token::RightParen {
            return Err("missing closing function parenthesis".into());
        }
        builtin(name, values)
    }
}

fn evaluate_expression(expression: &str, arguments: &Map<String, Value>) -> Result<Value, String> {
    if expression.len() > 16_384 {
        return Err("expression exceeds 16 KiB".into());
    }
    Parser::new(expression, arguments)?.parse()
}

fn builtin(name: &str, values: Vec<Value>) -> Result<Value, String> {
    match name {
        "concat" => Ok(Value::String(
            values.iter().map(display_value).collect::<String>(),
        )),
        "coalesce" => Ok(values
            .into_iter()
            .find(|value| !value.is_null())
            .unwrap_or(Value::Null)),
        "lower" | "upper" | "trim" => {
            let [value] = values.as_slice() else {
                return Err(format!("{name} expects one argument"));
            };
            let value = value
                .as_str()
                .ok_or_else(|| format!("{name} expects a string"))?;
            Ok(Value::String(match name {
                "lower" => value.to_lowercase(),
                "upper" => value.to_uppercase(),
                _ => value.trim().to_owned(),
            }))
        }
        "length" => {
            let [value] = values.as_slice() else {
                return Err("length expects one argument".into());
            };
            let length = value
                .as_str()
                .map(str::chars)
                .map(Iterator::count)
                .or_else(|| value.as_array().map(Vec::len))
                .ok_or("length expects a string or list")?;
            Ok(Value::Number(Number::from(length)))
        }
        _ => Err(format!("unknown controlled function '{name}'")),
    }
}

fn display_value(value: &Value) -> String {
    value.as_str().map_or_else(
        || {
            if value.is_null() {
                String::new()
            } else {
                value.to_string()
            }
        },
        str::to_owned,
    )
}
fn as_bool(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "boolean operator expects booleans".into())
}
fn as_number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .ok_or_else(|| "arithmetic expects numbers".into())
}
fn json_number(value: f64) -> Result<Value, String> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| "expression produced a non-finite number".into())
}
fn arithmetic(left: &Value, right: &Value, operator: &Token) -> Result<Value, String> {
    if operator == &Token::Plus && (left.is_string() || right.is_string()) {
        return Ok(Value::String(format!(
            "{}{}",
            display_value(left),
            display_value(right)
        )));
    }
    let left = as_number(left)?;
    let right = as_number(right)?;
    match operator {
        Token::Plus => json_number(left + right),
        Token::Minus => json_number(left - right),
        Token::Star => json_number(left * right),
        Token::Slash if right == 0.0 => Err("division by zero".into()),
        Token::Slash => json_number(left / right),
        _ => unreachable!(),
    }
}
fn compare_values(left: &Value, right: &Value, operator: &Token) -> Result<bool, String> {
    match operator {
        Token::Eq => Ok(left == right),
        Token::NotEq => Ok(left != right),
        Token::Greater | Token::GreaterEq | Token::Less | Token::LessEq => {
            let ordering = if let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) {
                left.partial_cmp(&right)
            } else if let (Some(left), Some(right)) = (left.as_str(), right.as_str()) {
                Some(left.cmp(right))
            } else {
                None
            }
            .ok_or("comparison expects two numbers or two strings")?;
            Ok(match operator {
                Token::Greater => ordering.is_gt(),
                Token::GreaterEq => ordering.is_ge(),
                Token::Less => ordering.is_lt(),
                Token::LessEq => ordering.is_le(),
                _ => unreachable!(),
            })
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_controlled_expressions() {
        let arguments =
            serde_json::from_value(serde_json::json!({"name":"  Billing ","count":3})).unwrap();
        assert_eq!(
            evaluate_expression("concat(upper(trim(name)), ':', count + 2)", &arguments).unwrap(),
            Value::String("BILLING:5.0".into())
        );
        assert_eq!(
            evaluate_expression("count >= 3 && name != ''", &arguments).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn rejects_unknown_expression_functions() {
        let error = evaluate_expression("shell('nope')", &Map::new()).unwrap_err();
        assert!(error.contains("unknown controlled function"));
    }

    #[test]
    fn executes_sandboxed_wasm_json_contract() {
        let wasm = wat::parse_str(r#"(module
          (memory (export "memory") 1 2)
          (global $heap (mut i32) (i32.const 1024))
          (func (export "alloc") (param $len i32) (result i32)
            (local $ptr i32) global.get $heap local.tee $ptr local.get $len i32.add global.set $heap local.get $ptr)
          (data (i32.const 16) "\22wasm-ok\22")
          (func (export "run") (param i32 i32) (result i64)
            i64.const 68719476745)
        )"#).unwrap();
        assert_eq!(
            execute_wasm(&wasm, "run", br"{}").unwrap(),
            Value::String("wasm-ok".into())
        );
    }
}
