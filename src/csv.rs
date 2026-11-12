//! 播放列表的 CSV：逗号分隔，字段可以用双引号包住（里面的 `""` 是一个引号，换行照留），行尾 `\n` 或 `\r\n`，
//! 开头可以有 UTF-8 的 BOM。空行去掉。

pub fn parse(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let finish_row = |row: &mut Vec<String>, field: &mut String, rows: &mut Vec<Vec<String>>| {
        row.push(std::mem::take(field));
        if !(row.len() == 1 && row[0].is_empty()) {
            rows.push(std::mem::take(row));
        } else {
            row.clear();
        }
    };
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => finish_row(&mut row, &mut field, &mut rows),
            (false, c) => field.push(c),
        }
    }
    if quoted {
        return Err("a quoted field is not closed".into());
    }
    if !field.is_empty() || !row.is_empty() {
        finish_row(&mut row, &mut field, &mut rows);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn rows_quotes_and_line_ends() {
        let rows =
            parse(b"\xef\xbb\xbfa,b\r\n\"x,1\",\"he said \"\"hi\"\"\"\r\n\r\nlast,\n").unwrap();
        assert_eq!(
            rows,
            [
                vec!["a", "b"],
                vec!["x,1", "he said \"hi\""],
                vec!["last", ""]
            ]
        );
        assert_eq!(
            parse(b"a,\"two\nlines\"\n").unwrap(),
            [vec!["a", "two\nlines"]]
        );
        assert_eq!(parse(b"").unwrap(), Vec::<Vec<String>>::new());
        assert_eq!(
            parse(b"no newline,at end").unwrap(),
            [vec!["no newline", "at end"]]
        );
    }

    #[test]
    fn refuses_what_is_not_csv_text() {
        assert!(parse(b"a,\"never closed\n").is_err());
        assert!(parse(b"\xff\xfe").is_err());
    }
}
