use std::path::Path;

/// OBJ 文件解析统计结果
#[derive(Debug, PartialEq, Eq)]
pub struct ObjParseResult {
    pub vertex_count: usize,
    pub face_count: usize,
    pub group_count: usize,
}

/// 解析 OBJ 文件，统计顶点、面、组数量
pub fn parse_obj_file(path: &Path) -> std::io::Result<ObjParseResult> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_obj_content(&content))
}

/// 解析 OBJ 内容字符串
pub fn parse_obj_content(content: &str) -> ObjParseResult {
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut group_count = 0usize;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("v ") {
            vertex_count += 1;
        } else if trimmed.starts_with("f ") {
            face_count += 1;
        } else if trimmed.starts_with("g ") {
            group_count += 1;
        }
    }

    ObjParseResult {
        vertex_count,
        face_count,
        group_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_obj_content_empty() {
        let result = parse_obj_content("");
        assert_eq!(
            result,
            ObjParseResult {
                vertex_count: 0,
                face_count: 0,
                group_count: 0
            }
        );
    }

    #[test]
    fn test_parse_obj_content_basic() {
        let content = "\
# OBJ file
g group1
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
f 1 2 3
g group2
v 0.0 0.0 1.0
f 1 2 4
";
        let result = parse_obj_content(content);
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.face_count, 2);
        assert_eq!(result.group_count, 2);
    }
}
