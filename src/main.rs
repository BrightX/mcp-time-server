use chrono::{DateTime, Datelike, NaiveTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use rmcp::{
    ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

// ---------- 输入参数（schema 来自 JsonSchema derive） ----------

#[derive(Debug, Deserialize)]
pub struct GetCurrentTimeArgs {
    /// IANA 时区名称，例如 'America/New_York'。不提供时使用系统本地时区
    #[serde(default)]
    pub timezone: Option<String>,
}

// 手动实现，避免 schemars 为 Option<String> 生成 "type": ["string","null"]
impl schemars::JsonSchema for GetCurrentTimeArgs {
    fn schema_name() -> Cow<'static, str> {
        "GetCurrentTimeArgs".into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "IANA 时区名称，例如 'America/New_York'。不提供时使用系统本地时区"
                }
            }
        })).expect("静态 JSON schema 一定是合法的")
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConvertTimeArgs {
    /// 源时区 IANA 名称
    pub source_timezone: String,
    /// 要转换的时间，格式 HH:MM（24 小时制）
    pub time: String,
    /// 目标时区 IANA 名称
    pub target_timezone: String,
}

// ---------- 输出结构 ----------

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TimeResult {
    pub timezone: String,
    pub datetime: String,
    pub is_dst: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TimeConversionResult {
    pub source: TimeResult,
    pub target: TimeResult,
    pub time_difference: String,
}

// ---------- 辅助函数 ----------

/// 判断某时刻是否处于夏令时：
/// DST 生效时偏移量一定大于当年 1 月/7 月偏移中的较小者（标准时偏移），
/// 同时兼容南半球（如悉尼 1 月是 DST）。
fn is_dst(tz: Tz, dt: &DateTime<Tz>) -> bool {
    let year = dt.year();
    let jan = Utc
        .with_ymd_and_hms(year, 1, 15, 12, 0, 0)
        .unwrap()
        .with_timezone(&tz);
    let jul = Utc
        .with_ymd_and_hms(year, 7, 15, 12, 0, 0)
        .unwrap()
        .with_timezone(&tz);
    let std_off = jan
        .offset()
        .fix()
        .local_minus_utc()
        .min(jul.offset().fix().local_minus_utc());
    dt.offset().fix().local_minus_utc() > std_off
}

// 对应 Python: raise ValueError(f"Invalid timezone: ...")
fn parse_tz(name: &str) -> Result<Tz, ErrorData> {
    name.parse::<Tz>()
        .map_err(|_| ErrorData::invalid_params(format!("Invalid timezone: '{name}'"), None))
}

fn json_result<T: Serialize>(value: T) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| ErrorData::internal_error(format!("serialize failed: {e}"), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

// ---------- 服务器 ----------

#[derive(Clone)]
pub struct TimeServer {
    tool_router: ToolRouter<TimeServer>,
}

// 说明：文档示例中路由可以是自由函数（不存字段），
// 但 ServerHandler::call_tool 需要通过 Self 访问 router，
// 存字段 + tool_handler(router = self.tool_router) 是组合多路由/自定义结构时的稳妥写法。
// 如果你的版本支持默认路由（不存字段），见文末的“极简版”。
#[tool_router]
impl TimeServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "get_current_time",
        description = "Get current time in a specific timezone. If timezone is omitted, uses the system local timezone."
    )]
    async fn get_current_time(
        &self,
        Parameters(args): Parameters<GetCurrentTimeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        // 对应 Python: if timezone: ... else: 用本地时区
        let tz_name = match args.timezone {
            Some(name) => name,
            None => iana_time_zone::get_timezone().map_err(|e| {
                ErrorData::internal_error(format!("无法获取系统本地时区: {e}"), None)
            })?,
        };

        let tz = parse_tz(&tz_name)?;
        let now = Utc::now().with_timezone(&tz);

        json_result(TimeResult {
            timezone: tz_name,
            datetime: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            is_dst: is_dst(tz, &now),
        })
    }

    #[tool(name = "convert_time", description = "Convert time between timezones")]
    async fn convert_time(
        &self,
        Parameters(args): Parameters<ConvertTimeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let source_tz = parse_tz(&args.source_timezone)?;
        let target_tz = parse_tz(&args.target_timezone)?;

        // 对应 Python: datetime.strptime(time, "%H:%M").time()
        let parsed_time = NaiveTime::parse_from_str(&args.time, "%H:%M").map_err(|_| {
            ErrorData::invalid_params(
                format!(
                    "Invalid time format. Expected HH:MM in 24hr format, got '{}'",
                    args.time
                ),
                None,
            )
        })?;

        // 用源时区的“今天”与给定时间拼出本地时间
        let source_dt = Utc::now()
            .with_timezone(&source_tz)
            .date_naive()
            .and_time(parsed_time)
            .and_local_timezone(source_tz)
            .earliest()
            .ok_or_else(|| ErrorData::invalid_params("无法在源时区解析该时间", None))?;

        let target_dt = source_dt.with_timezone(&target_tz);

        // 对应 Python: (target.utcoffset() - source.utcoffset()).total_seconds() / 3600
        let diff_seconds = (target_dt.offset().fix().local_minus_utc()
            - source_dt.offset().fix().local_minus_utc()) as f64;
        let diff_hours = diff_seconds / 3600.0;

        json_result(TimeConversionResult {
            source: TimeResult {
                timezone: args.source_timezone,
                datetime: source_dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                is_dst: is_dst(source_tz, &source_dt),
            },
            target: TimeResult {
                timezone: args.target_timezone,
                datetime: target_dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                is_dst: is_dst(target_tz, &target_dt),
            },
            // 对应 Python: f"{diff:+}h"，如 +5.75h、+1h、-3h
            time_difference: format!("{diff_hours:+}h"),
        })
    }
}

// 文档推荐的写法：工具型服务器让宏自动生成 get_info
// （名称/版本取自 Cargo.toml）。需要自定义 instructions 时用属性参数：
#[tool_handler(
    router = self.tool_router,
    name = "mcp-time",
    version = "1.0.0",
    instructions = "Get current time or convert between timezones"
)]
impl ServerHandler for TimeServer {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = TimeServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
