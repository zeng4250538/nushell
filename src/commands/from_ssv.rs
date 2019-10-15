use crate::commands::WholeStreamCommand;
use crate::data::{Primitive, TaggedDictBuilder, Value};
use crate::prelude::*;

pub struct FromSSV;

#[derive(Deserialize)]
pub struct FromSSVArgs {
    headerless: bool,
}

const STRING_REPRESENTATION: &str = "from-ssv";

impl WholeStreamCommand for FromSSV {
    fn name(&self) -> &str {
        STRING_REPRESENTATION
    }

    fn signature(&self) -> Signature {
        Signature::build(STRING_REPRESENTATION).switch("headerless")
    }

    fn usage(&self) -> &str {
        "Parse text as whitespace-separated values and create a table."
    }

    fn run(
        &self,
        args: CommandArgs,
        registry: &CommandRegistry,
    ) -> Result<OutputStream, ShellError> {
        args.process(registry, from_ssv)?.run()
    }
}

fn string_to_table(s: &str, headerless: bool) -> Option<Vec<Vec<(String, String)>>> {
    let mut lines = s.lines().filter(|l| !l.trim().is_empty());

    let headers = lines
        .next()?
        .split_whitespace()
        .map(|s| s.to_owned())
        .collect::<Vec<String>>();

    let header_row = if headerless {
        (1..=headers.len())
            .map(|i| format!("Column{}", i))
            .collect::<Vec<String>>()
    } else {
        headers
    };

    Some(
        lines
            .map(|l| {
                header_row
                    .iter()
                    .zip(l.split_whitespace())
                    .map(|(a, b)| (String::from(a), String::from(b)))
                    .collect()
            })
            .collect(),
    )
}

fn from_ssv_string_to_value(
    s: &str,
    headerless: bool,
    tag: impl Into<Tag>,
) -> Option<Tagged<Value>> {
    let tag = tag.into();
    let rows = string_to_table(s, headerless)?
        .iter()
        .map(|row| {
            let mut tagged_dict = TaggedDictBuilder::new(&tag);
            for (col, entry) in row {
                tagged_dict.insert_tagged(
                    col,
                    Value::Primitive(Primitive::String(String::from(entry))).tagged(&tag),
                )
            }
            tagged_dict.into_tagged_value()
        })
        .collect();

    Some(Value::Table(rows).tagged(&tag))
}

fn from_ssv(
    FromSSVArgs { headerless }: FromSSVArgs,
    RunnableContext { input, name, .. }: RunnableContext,
) -> Result<OutputStream, ShellError> {
    let stream = async_stream! {
        let values: Vec<Tagged<Value>> = input.values.collect().await;
        let mut concat_string = String::new();
        let mut latest_tag: Option<Tag> = None;

        for value in values {
            let value_tag = value.tag();
            latest_tag = Some(value_tag.clone());
            match value.item {
                Value::Primitive(Primitive::String(s)) => {
                    concat_string.push_str(&s);
                }
                _ => yield Err(ShellError::labeled_error_with_secondary (
                    "Expected a string from pipeline",
                    "requires string input",
                    &name,
                    "value originates from here",
                    &value_tag
                )),
            }
        }

        match from_ssv_string_to_value(&concat_string, headerless, name.clone()) {
            Some(x) => match x {
                Tagged { item: Value::Table(list), ..} => {
                    for l in list { yield ReturnSuccess::value(l) }
                }
                x => yield ReturnSuccess::value(x)
            },
            None => if let Some(tag) = latest_tag {
                yield Err(ShellError::labeled_error_with_secondary(
                    "Could not parse as SSV",
                    "input cannot be parsed ssv",
                    &name,
                    "value originates from here",
                    &tag,
                ))
            },
        }
    };

    Ok(stream.to_output_stream())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owned(x: &str, y: &str) -> (String, String) {
        (String::from(x), String::from(y))
    }

    #[test]
    fn it_trims_empty_and_whitespace_only_lines() {
        let input = r#"

            a       b

            1    2

            3 4
        "#;
        let result = string_to_table(input, false);
        assert_eq!(
            result,
            Some(vec![
                vec![owned("a", "1"), owned("b", "2")],
                vec![owned("a", "3"), owned("b", "4")]
            ])
        );
    }

    #[test]
    fn it_ignores_headers_when_headerless() {
        let input = r#"
            a b
            1 2
            3 4
        "#;
        let result = string_to_table(input, true);
        assert_eq!(
            result,
            Some(vec![
                vec![owned("Column1", "1"), owned("Column2", "2")],
                vec![owned("Column1", "3"), owned("Column2", "4")]
            ])
        );
    }

    #[test]
    fn it_returns_none_given_an_empty_string() {
        let input = "";
        let result = string_to_table(input, true);
        assert_eq!(result, None);
    }

    #[test]
    fn it_allows_a_predefined_number_of_spaces() {
        let input = r#"
            column a   column b
            entry 1   entry number  2
            3   four
        "#;

        let result = string_to_table(input, false);
        assert_eq!(
            result,
            Some(vec![
                vec![
                    owned("column a", "entry 1"),
                    owned("column b", "entry number  2")
                ],
                vec![owned("column a", "3"), owned("column b", "four")]
            ])
        );
    }

    #[test]
    fn it_trims_remaining_separator_space() {
        let input = r#"
            colA   colB     colC
            val1   val2     val3
        "#;

        let trimmed = |s: &str| s.trim() == s;

        let result = string_to_table(input, false).unwrap();
        assert_eq!(
            true,
            result
                .iter()
                .all(|row| row.iter().all(|(a, b)| trimmed(a) && trimmed(b)))
        )
    }
}
