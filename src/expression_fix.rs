use aios_core::{CataContext, eval_str_to_f64};
use anyhow::{Result, anyhow};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use regex::Regex;

/// 表达式验证错误信息
#[derive(Debug, Clone)]
pub struct ExpressionValidationError {
    /// 原始表达式
    pub expression: String,
    /// 属性名称（如 PRAD, PHEI 等）
    pub attr_name: String,
    /// 几何体 refno
    pub gm_refno: String,
    /// 几何体类型
    pub gm_type: String,
    /// 错误类型
    pub error_type: ExpressionErrorType,
    /// 详细错误信息
    pub message: String,
}

/// 表达式错误类型
#[derive(Debug, Clone)]
pub enum ExpressionErrorType {
    /// 括号不匹配
    UnbalancedBrackets { left: usize, right: usize },
    /// 空表达式
    EmptyExpression,
    /// 无效的 ATTRIB 格式
    InvalidAttribFormat,
    /// 未知变量
    UnknownVariable(String),
    /// 其他语法错误
    SyntaxError(String),
}

impl std::fmt::Display for ExpressionValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[表达式错误] 几何体={} ({}), 属性={}, 表达式='{}', 错误: {}",
            self.gm_refno, self.gm_type, self.attr_name, self.expression, self.message
        )
    }
}

/// 表达式修复器 - 专门处理ATTRIB关键字和相关表达式问题
pub struct ExpressionFixer;

impl ExpressionFixer {
    /// 规整 `ATTRIB :NAME` 这种 UDA 写法（去掉冒号），避免下游解析器不识别 `:` 前缀。
    ///
    /// 例如：`ATTRIB :HXYS[1 ]` -> `ATTRIB HXYS[1 ]`
    pub fn normalize_attrib_colon(expr: &str) -> String {
        static ATTRIB_COLON_REGEX: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"ATTRIB\s+:\s*").expect("invalid ATTRIB_COLON_REGEX"));
        ATTRIB_COLON_REGEX.replace_all(expr, "ATTRIB ").to_string()
    }

    /// 预处理ATTRIB表达式，将ATTRIB关键字转换为可解析的变量名
    ///
    /// 注意：函数名会转换为小写，因为 tiny_expr 解析器只识别小写函数名
    /// （如 sqrt, pow, sin, cos 等），但变量名保持原样（大写）
    pub fn preprocess_attrib_expression(expr: &str) -> String {
        // 处理 ATTRIB PARA[数字] 格式，支持空格（大小写不敏感）
        let attrib_para_regex = Regex::new(r"(?i)ATTRIB\s+PARA\s*\[\s*(\d+)\s*\]").unwrap();
        let mut processed = attrib_para_regex.replace_all(expr, "PARA$1").to_string();

        // 处理 ATTRIB 属性名 格式（大小写不敏感）
        let attrib_regex = Regex::new(r"(?i)ATTRIB\s+([A-Za-z][A-Za-z0-9_]*)").unwrap();
        processed = attrib_regex.replace_all(&processed, "$1").to_string();

        // 将已知的数学函数名转换为小写（tiny_expr 只识别小写函数名）
        // 包括：SQRT, POW, SIN, COS, TAN, ASIN, ACOS, ATAN, ATAN2, LOG, EXP, ABS,
        //       MIN, MAX, CEIL, FLOOR, ROUND, INT, LN, LOG10, SINH, COSH, TANH
        processed = Self::lowercase_function_names(&processed);

        // 清理多余空格和格式化
        processed = processed
            .replace("  ", " ")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(" ,", ",")
            .replace(", ", ",")
            .trim()
            .to_string();

        processed
    }

    /// 将已知的数学函数名转换为小写
    /// 这是必要的，因为 tiny_expr 解析器只识别小写的函数名
    fn lowercase_function_names(expr: &str) -> String {
        // 已知的函数名列表（tiny_expr 支持的函数）
        static FUNCTION_NAMES: &[&str] = &[
            "SQRT", "POW", "SIN", "COS", "TAN", "ASIN", "ACOS", "ATAN", "ATAN2", "ATANT",
            "LOG", "LOG10", "LN", "EXP", "ABS", "MIN", "MAX", "CEIL", "FLOOR", "ROUND",
            "INT", "NINT", "SINH", "COSH", "TANH", "PI", "E", "RAND01", "RANDINT",
        ];

        let mut result = expr.to_string();

        for func_name in FUNCTION_NAMES {
            // 使用正则表达式匹配函数名后跟左括号的模式
            // 这样可以避免误匹配变量名
            let pattern = format!(r"(?i)\b{}\s*\(", regex::escape(func_name));
            if let Ok(re) = Regex::new(&pattern) {
                result = re.replace_all(&result, |caps: &regex::Captures| {
                    let matched = &caps[0];
                    matched.to_lowercase()
                }).to_string();
            }
        }

        result
    }

    /// 增强的表达式求值函数，支持ATTRIB关键字
    pub fn eval_expression_with_attrib_support(
        expr: &str,
        context: &CataContext,
        unit: &str,
    ) -> Result<f64> {
        // 第一步：预处理ATTRIB关键字
        let processed_expr = Self::preprocess_attrib_expression(expr);

        // 第二步：验证处理后的表达式
        if processed_expr.is_empty() {
            return Err(anyhow!("表达式为空"));
        }

        // 第三步：调用原有的表达式解析
        match eval_str_to_f64(&processed_expr, context, unit) {
            Ok(result) => Ok(result),
            Err(e) => {
                // 提供详细的错误信息
                let error_analysis = Self::analyze_expression_error(&e, expr, &processed_expr);
                Err(anyhow!(
                    "表达式解析失败:\n原始表达式: {}\n处理后表达式: {}\n错误: {}\n建议: {}",
                    expr,
                    processed_expr,
                    e,
                    error_analysis.join("; ")
                ))
            }
        }
    }

    /// 分析表达式错误并提供解决建议
    fn analyze_expression_error(
        error: &anyhow::Error,
        original_expr: &str,
        processed_expr: &str,
    ) -> Vec<String> {
        let error_msg = error.to_string().to_lowercase();
        let mut suggestions = Vec::new();

        // 检查ATTRIB相关问题
        if original_expr.contains("ATTRIB") {
            suggestions.push("ATTRIB关键字已预处理，检查属性名是否在上下文中定义".to_string());

            if original_expr.contains("PARA[") {
                suggestions.push("PARA数组索引已转换，确认索引值是否正确".to_string());
            }
        }

        // 检查函数相关问题
        if error_msg.contains("min") || error_msg.contains("max") {
            suggestions.push("MIN/MAX函数需要两个参数，检查参数数量".to_string());
        }

        // 检查语法问题
        if error_msg.contains("parse") || error_msg.contains("syntax") {
            suggestions.push("检查括号是否配对".to_string());
            suggestions.push("验证操作符和函数名拼写".to_string());
        }

        // 检查变量问题
        if error_msg.contains("variable") || error_msg.contains("undefined") {
            suggestions.push("检查所有变量是否在上下文中定义".to_string());
        }

        // 通用建议
        if suggestions.is_empty() {
            suggestions.push("检查表达式语法格式".to_string());
            suggestions.push("验证所有变量和函数名".to_string());
        }

        suggestions
    }

    /// 创建测试上下文，用于验证表达式修复
    /// 注意：变量名使用大写，与 PDMS 数据库中的格式一致
    pub fn create_test_context() -> CataContext {
        let mut context = DashMap::new();

        // 添加常用的测试属性（大写，与 PDMS 格式一致）
        context.insert("HEIG".to_string(), "100.0".to_string());
        context.insert("PARA0".to_string(), "10.0".to_string());
        context.insert("PARA1".to_string(), "20.0".to_string());
        context.insert("PARA2".to_string(), "30.0".to_string());
        context.insert("PARA3".to_string(), "40.0".to_string());
        context.insert("PARA4".to_string(), "50.0".to_string());
        context.insert("PARA5".to_string(), "60.0".to_string());

        // 添加其他常用属性
        context.insert("WIDT".to_string(), "200.0".to_string());
        context.insert("LENG".to_string(), "300.0".to_string());
        context.insert("RADI".to_string(), "25.0".to_string());

        CataContext {
            context,
            is_tubi: false,
            ..Default::default()
        }
    }

    /// 验证表达式语法（括号配对等）
    /// 返回 Ok(()) 如果表达式有效，否则返回错误信息
    pub fn validate_expression_syntax(expr: &str) -> Result<(), ExpressionErrorType> {
        if expr.trim().is_empty() || expr == "UNSET" {
            return Ok(()); // 空表达式和 UNSET 是允许的
        }

        // 检查括号配对
        let mut bracket_count = 0i32;
        let mut square_bracket_count = 0i32;
        
        for ch in expr.chars() {
            match ch {
                '(' => bracket_count += 1,
                ')' => {
                    bracket_count -= 1;
                    if bracket_count < 0 {
                        return Err(ExpressionErrorType::UnbalancedBrackets {
                            left: expr.matches('(').count(),
                            right: expr.matches(')').count(),
                        });
                    }
                }
                '[' => square_bracket_count += 1,
                ']' => {
                    square_bracket_count -= 1;
                    if square_bracket_count < 0 {
                        return Err(ExpressionErrorType::SyntaxError(
                            "方括号不匹配：多余的 ']'".to_string()
                        ));
                    }
                }
                _ => {}
            }
        }

        if bracket_count != 0 {
            return Err(ExpressionErrorType::UnbalancedBrackets {
                left: expr.matches('(').count(),
                right: expr.matches(')').count(),
            });
        }

        if square_bracket_count != 0 {
            return Err(ExpressionErrorType::SyntaxError(
                "方括号不匹配".to_string()
            ));
        }

        Ok(())
    }

    /// 验证 GmParam 中的所有表达式
    /// 返回所有发现的错误列表
    pub fn validate_gm_param_expressions(
        gm_refno: &str,
        gm_type: &str,
        expressions: &[(&str, &str)], // (属性名, 表达式)
    ) -> Vec<ExpressionValidationError> {
        let mut errors = Vec::new();

        for (attr_name, expr) in expressions {
            if let Err(error_type) = Self::validate_expression_syntax(expr) {
                let message = match &error_type {
                    ExpressionErrorType::UnbalancedBrackets { left, right } => {
                        format!("括号不匹配：左括号 {} 个，右括号 {} 个", left, right)
                    }
                    ExpressionErrorType::EmptyExpression => "空表达式".to_string(),
                    ExpressionErrorType::InvalidAttribFormat => "无效的 ATTRIB 格式".to_string(),
                    ExpressionErrorType::UnknownVariable(var) => format!("未知变量: {}", var),
                    ExpressionErrorType::SyntaxError(msg) => msg.clone(),
                };

                errors.push(ExpressionValidationError {
                    expression: expr.to_string(),
                    attr_name: attr_name.to_string(),
                    gm_refno: gm_refno.to_string(),
                    gm_type: gm_type.to_string(),
                    error_type,
                    message,
                });
            }
        }

        errors
    }
}

/// 评估PDMS表达式 - 支持ATTRIB关键字和数组索引语法
pub fn eval_pdms_expression(expr: &str, context: &CataContext) -> Result<f64> {
    ExpressionFixer::eval_expression_with_attrib_support(expr, context, "DIST")
}

/// 评估ATTRIB表达式 - 专门处理ATTRIB关键字
pub fn eval_attrib_expression(expr: &str, context: &CataContext, unit: &str) -> Result<f64> {
    ExpressionFixer::eval_expression_with_attrib_support(expr, context, unit)
}

/// 评估增强表达式 - 通用的增强表达式评估器
pub fn eval_enhanced_expression(expr: &str, context: &CataContext, unit: &str) -> Result<f64> {
    ExpressionFixer::eval_expression_with_attrib_support(expr, context, unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_attrib_expression() {
        // 测试ATTRIB PARA[数字]格式 - 函数名应为小写，变量名保持大写
        // 注意：函数名和左括号之间可能有空格
        let expr1 = "( MIN ( ATTRIB HEIG,ATTRIB PARA[3 ] ) )";
        let processed1 = ExpressionFixer::preprocess_attrib_expression(expr1);
        // 预期结果：函数名小写，变量名大写，空格清理后
        assert!(processed1.contains("min(") || processed1.contains("min ("),
                "Expected lowercase 'min', got: {}", processed1);
        assert!(processed1.contains("HEIG"), "Expected uppercase 'HEIG', got: {}", processed1);
        assert!(processed1.contains("PARA3"), "Expected 'PARA3', got: {}", processed1);

        // 测试简单ATTRIB格式 - 变量名保持大写
        let expr2 = "ATTRIB WIDT + ATTRIB LENG";
        let processed2 = ExpressionFixer::preprocess_attrib_expression(expr2);
        assert_eq!(processed2, "WIDT + LENG");

        // 测试复杂表达式 - 函数名小写，变量名大写
        let expr3 = "MAX(ATTRIB PARA[0], ATTRIB PARA[1] * 2)";
        let processed3 = ExpressionFixer::preprocess_attrib_expression(expr3);
        assert!(processed3.contains("max("), "Expected lowercase 'max(', got: {}", processed3);
        assert!(processed3.contains("PARA0"), "Expected 'PARA0', got: {}", processed3);
        assert!(processed3.contains("PARA1"), "Expected 'PARA1', got: {}", processed3);
    }

    #[test]
    fn test_preprocess_sqrt_pow_functions() {
        // 测试 SQRT 函数（大写）转换为小写
        let expr1 = "SQRT( 16 )";
        let processed1 = ExpressionFixer::preprocess_attrib_expression(expr1);
        assert!(processed1.starts_with("sqrt("), "Expected 'sqrt(', got: {}", processed1);

        // 测试 POW 函数（大写）转换为小写
        let expr2 = "POW( 2, 3 )";
        let processed2 = ExpressionFixer::preprocess_attrib_expression(expr2);
        assert!(processed2.starts_with("pow("), "Expected 'pow(', got: {}", processed2);

        // 测试复杂的 SQRT/POW 组合表达式 - 函数名小写，变量名大写
        let expr3 = "SQRT( POW( ATTRIB PARA[2], 2 ) + POW( ATTRIB PARA[3], 2 ) )";
        let processed3 = ExpressionFixer::preprocess_attrib_expression(expr3);
        assert!(processed3.contains("sqrt("), "Expected 'sqrt(', got: {}", processed3);
        assert!(processed3.contains("pow("), "Expected 'pow(', got: {}", processed3);
        assert!(processed3.contains("PARA2"), "Expected 'PARA2', got: {}", processed3);
        assert!(processed3.contains("PARA3"), "Expected 'PARA3', got: {}", processed3);

        // 测试实际问题表达式（来自 RUS-149）- 变量名保持大写
        let expr4 = "( ( SQRT( 3 ) * ATTRIB PARA[9 ] ) / 2 )";
        let processed4 = ExpressionFixer::preprocess_attrib_expression(expr4);
        assert!(processed4.contains("sqrt("), "Expected 'sqrt(', got: {}", processed4);
        assert!(processed4.contains("PARA9"), "Expected 'PARA9', got: {}", processed4);
    }

    #[test]
    fn test_lowercase_function_names() {
        // 测试各种函数名转换
        let expr = "SIN(45) + COS(45) + TAN(45)";
        let result = ExpressionFixer::lowercase_function_names(expr);
        assert_eq!(result, "sin(45) + cos(45) + tan(45)");

        // 测试不转换变量名（变量名后面没有括号）
        let expr2 = "SQRT + POW";  // 这些是变量名，不是函数调用
        let result2 = ExpressionFixer::lowercase_function_names(expr2);
        assert_eq!(result2, "SQRT + POW");  // 不应该被转换

        // 测试函数名后有空格再有括号
        // 注意：函数名和括号之间的空格会被保留（转小写时保持原样）
        // 后续的 preprocess_attrib_expression 会清理这些空格
        let expr3 = "SQRT  ( 16 )";
        let result3 = ExpressionFixer::lowercase_function_names(expr3);
        assert!(result3.contains("sqrt"), "应该将 SQRT 转为小写: {}", result3);
        assert!(result3.contains("( 16 )"), "应该保留括号和内容: {}", result3);
    }

    #[test]
    fn test_eval_expression_with_attrib_support() {
        let context = ExpressionFixer::create_test_context();

        // 测试原始问题表达式
        let expr = "( MIN ( ATTRIB HEIG,ATTRIB PARA[3 ] ) )";
        let result = ExpressionFixer::eval_expression_with_attrib_support(expr, &context, "DIST");

        assert!(result.is_ok(), "Expression evaluation failed: {:?}", result);
        assert_eq!(result.unwrap(), 40.0); // MIN(100.0, 40.0) = 40.0
    }

    #[test]
    fn test_eval_sqrt_function() {
        let context = ExpressionFixer::create_test_context();

        // 测试 SQRT 函数（大写输入）
        let expr = "SQRT(16)";
        let result = ExpressionFixer::eval_expression_with_attrib_support(expr, &context, "DIST");
        assert!(result.is_ok(), "SQRT(16) failed: {:?}", result);
        assert_eq!(result.unwrap(), 4.0);

        // 测试 SQRT(3) - 来自实际问题表达式
        // 注意：eval_str_to_f64 会对结果进行 f64_round_3 处理（四舍五入到3位小数）
        let expr2 = "SQRT(3)";
        let result2 = ExpressionFixer::eval_expression_with_attrib_support(expr2, &context, "DIST");
        assert!(result2.is_ok(), "SQRT(3) failed: {:?}", result2);
        let expected = 1.732; // sqrt(3) ≈ 1.732 (四舍五入到3位小数)
        let actual = result2.unwrap();
        assert!((actual - expected).abs() < 1e-6, "SQRT(3): actual = {}, expected = {}", actual, expected);
    }

    #[test]
    fn test_eval_pow_function() {
        let context = ExpressionFixer::create_test_context();

        // 测试 POW 函数（大写输入）
        let expr = "POW(2, 3)";
        let result = ExpressionFixer::eval_expression_with_attrib_support(expr, &context, "DIST");
        assert!(result.is_ok(), "POW(2, 3) failed: {:?}", result);
        assert_eq!(result.unwrap(), 8.0);

        // 测试 POW 与 ATTRIB 结合
        let expr2 = "POW(ATTRIB PARA[1], 2)"; // PARA1 = 20.0
        let result2 = ExpressionFixer::eval_expression_with_attrib_support(expr2, &context, "DIST");
        assert!(result2.is_ok(), "POW(PARA1, 2) failed: {:?}", result2);
        assert_eq!(result2.unwrap(), 400.0); // 20^2 = 400
    }

    #[test]
    fn test_eval_complex_sqrt_pow_expression() {
        let mut context = DashMap::new();
        context.insert("PARA2".to_string(), "3.0".to_string());
        context.insert("PARA3".to_string(), "4.0".to_string());
        let cata_context = CataContext {
            context,
            is_tubi: false,
            ..Default::default()
        };

        // 测试 SQRT( POW(a,2) + POW(b,2) ) = sqrt(3^2 + 4^2) = sqrt(9+16) = sqrt(25) = 5
        let expr = "SQRT( POW( ATTRIB PARA[2], 2 ) + POW( ATTRIB PARA[3], 2 ) )";
        let result = ExpressionFixer::eval_expression_with_attrib_support(expr, &cata_context, "DIST");
        assert!(result.is_ok(), "Complex SQRT/POW expression failed: {:?}", result);
        assert_eq!(result.unwrap(), 5.0);
    }

    #[test]
    fn test_max_function() {
        let context = ExpressionFixer::create_test_context();

        let expr = "MAX(ATTRIB PARA[1], ATTRIB PARA[2])";
        let result = eval_pdms_expression(expr, &context);

        assert!(result.is_ok(), "MAX function failed: {:?}", result);
        assert_eq!(result.unwrap(), 30.0); // MAX(20.0, 30.0) = 30.0
    }

    #[test]
    fn test_trig_functions_uppercase() {
        let context = ExpressionFixer::create_test_context();

        // 测试 SIN 函数（大写）
        let expr_sin = "SIN(90)";
        let result_sin = ExpressionFixer::eval_expression_with_attrib_support(expr_sin, &context, "DIST");
        assert!(result_sin.is_ok(), "SIN(90) failed: {:?}", result_sin);
        // SIN(90度) = 1.0 (tiny_expr 使用度数)
        assert!((result_sin.unwrap() - 1.0).abs() < 1e-10);

        // 测试 COS 函数（大写）
        let expr_cos = "COS(0)";
        let result_cos = ExpressionFixer::eval_expression_with_attrib_support(expr_cos, &context, "DIST");
        assert!(result_cos.is_ok(), "COS(0) failed: {:?}", result_cos);
        assert!((result_cos.unwrap() - 1.0).abs() < 1e-10);

        // 测试 TAN 函数（大写）
        let expr_tan = "TAN(45)";
        let result_tan = ExpressionFixer::eval_expression_with_attrib_support(expr_tan, &context, "DIST");
        assert!(result_tan.is_ok(), "TAN(45) failed: {:?}", result_tan);
        assert!((result_tan.unwrap() - 1.0).abs() < 1e-10);
    }
}
