//! MCP Prompts for AI-assisted SQL operations.
//!
//! Prompts are templates that help LLMs generate contextually-aware SQL queries
//! by providing schema information and best practices.

use crate::server::MssqlMcpServer;
use rmcp::model::{
    GetPromptResult, Prompt, PromptArgument, PromptMessage, PromptMessageContent,
    PromptMessageRole,
};
use std::collections::HashMap;

/// Create a prompt argument helper.
fn prompt_arg(name: &str, description: &str, required: bool) -> PromptArgument {
    PromptArgument {
        name: name.to_string(),
        title: None,
        description: Some(description.to_string()),
        required: Some(required),
    }
}

/// Create a prompt helper.
fn prompt(name: &str, description: &str, arguments: Vec<PromptArgument>) -> Prompt {
    Prompt {
        name: name.to_string(),
        title: None,
        description: Some(description.to_string()),
        arguments: Some(arguments),
        icons: None,
        meta: None,
    }
}

/// Build the list of available prompts.
pub fn build_prompt_list() -> Vec<Prompt> {
    vec![
        prompt(
            "query-table",
            "Generate a SELECT query for a table with its schema information",
            vec![
                prompt_arg("schema", "Schema name (default: dbo)", false),
                prompt_arg("table", "Table name to query", true),
                prompt_arg("columns", "Comma-separated list of columns (default: all)", false),
                prompt_arg("filter", "Natural language filter condition", false),
            ],
        ),
        prompt(
            "analyze-schema",
            "Analyze a table's schema and suggest optimizations or improvements",
            vec![
                prompt_arg("schema", "Schema name", false),
                prompt_arg("table", "Table name to analyze", true),
            ],
        ),
        prompt(
            "generate-insert",
            "Generate an INSERT statement template for a table",
            vec![
                prompt_arg("schema", "Schema name", false),
                prompt_arg("table", "Table name", true),
            ],
        ),
        prompt(
            "explain-procedure",
            "Explain what a stored procedure does and how to call it",
            vec![
                prompt_arg("schema", "Schema name", false),
                prompt_arg("procedure", "Procedure name", true),
            ],
        ),
        prompt(
            "optimize-query",
            "Analyze a SQL query and suggest optimizations",
            vec![prompt_arg("query", "SQL query to optimize", true)],
        ),
        prompt(
            "debug-error",
            "Help debug a SQL Server error with context and suggestions",
            vec![
                prompt_arg("error", "Error message or code", true),
                prompt_arg("query", "Query that caused the error (optional)", false),
            ],
        ),
    ]
}

/// Get a specific prompt with arguments filled in.
pub async fn get_prompt(
    server: &MssqlMcpServer,
    name: &str,
    arguments: Option<&HashMap<String, String>>,
) -> Result<GetPromptResult, String> {
    let args = arguments.cloned().unwrap_or_default();

    match name {
        "query-table" => get_query_table_prompt(server, &args).await,
        "analyze-schema" => get_analyze_schema_prompt(server, &args).await,
        "generate-insert" => get_generate_insert_prompt(server, &args).await,
        "explain-procedure" => get_explain_procedure_prompt(server, &args).await,
        "optimize-query" => get_optimize_query_prompt(&args),
        "debug-error" => get_debug_error_prompt(&args),
        _ => Err(format!("Unknown prompt: {}", name)),
    }
}

// =========================================================================
// Prompt Implementations
// =========================================================================

async fn get_query_table_prompt(
    server: &MssqlMcpServer,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, String> {
    let schema = args.get("schema").map(|s| s.as_str()).unwrap_or("dbo");
    let table = args
        .get("table")
        .ok_or("Missing required argument: table")?;
    let columns = args.get("columns");
    let filter = args.get("filter");

    // Get table columns from metadata
    let column_info = server
        .metadata
        .get_table_columns(schema, table)
        .await
        .map_err(|e| e.to_string())?;

    if column_info.is_empty() {
        return Err(format!("Table not found: {}.{}", schema, table));
    }

    // Build schema description
    let schema_desc = column_info
        .iter()
        .map(|c| {
            format!(
                "  - {} ({}{}){}",
                c.column_name,
                c.data_type,
                if c.is_nullable { ", nullable" } else { "" },
                if c.is_identity { " [IDENTITY]" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut prompt_text = format!(
        r#"Generate a SELECT query for the table [{schema}].[{table}].

## Table Schema

{schema_desc}

## Requirements
"#
    );

    if let Some(cols) = columns {
        prompt_text.push_str(&format!("- Select only these columns: {}\n", cols));
    } else {
        prompt_text.push_str("- Select all relevant columns\n");
    }

    if let Some(f) = filter {
        prompt_text.push_str(&format!("- Filter condition: {}\n", f));
    }

    prompt_text.push_str(
        r#"
## Guidelines
- Use proper bracket notation for identifiers: [schema].[table].[column]
- Add appropriate TOP or OFFSET/FETCH for large tables
- Consider using aliases for readability
- Include ORDER BY for deterministic results
"#,
    );

    Ok(GetPromptResult {
        description: Some(format!("Query builder for {}.{}", schema, table)),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}

async fn get_analyze_schema_prompt(
    server: &MssqlMcpServer,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, String> {
    let schema = args.get("schema").map(|s| s.as_str()).unwrap_or("dbo");
    let table = args
        .get("table")
        .ok_or("Missing required argument: table")?;

    // Get table columns
    let columns = server
        .metadata
        .get_table_columns(schema, table)
        .await
        .map_err(|e| e.to_string())?;

    if columns.is_empty() {
        return Err(format!("Table not found: {}.{}", schema, table));
    }

    let column_desc = columns
        .iter()
        .map(|c| {
            format!(
                "| {} | {} | {} | {} | {} | {} |",
                c.column_name,
                c.data_type,
                c.max_length.map(|l| l.to_string()).unwrap_or("-".to_string()),
                if c.is_nullable { "Yes" } else { "No" },
                if c.is_identity { "Yes" } else { "No" },
                c.default_value.as_deref().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_text = format!(
        r#"Analyze the schema of table [{schema}].[{table}] and provide recommendations.

## Current Schema

| Column | Type | Max Length | Nullable | Identity | Default |
|--------|------|------------|----------|----------|---------|
{column_desc}

## Analysis Requested

Please analyze this schema and provide:

1. **Data Type Review**
   - Are the data types appropriate for the column names/purposes?
   - Could any types be optimized (e.g., varchar(max) -> varchar(n))?

2. **Nullability Assessment**
   - Which nullable columns might benefit from NOT NULL constraints?
   - Are there potential data integrity issues?

3. **Indexing Suggestions**
   - Based on likely query patterns, which columns should be indexed?
   - Any composite index recommendations?

4. **Best Practices**
   - Naming convention compliance
   - Potential normalization issues
   - Missing audit columns (CreatedAt, UpdatedAt, etc.)

5. **Performance Considerations**
   - Data type sizes and storage efficiency
   - Potential query performance impacts
"#
    );

    Ok(GetPromptResult {
        description: Some(format!("Schema analysis for {}.{}", schema, table)),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}

async fn get_generate_insert_prompt(
    server: &MssqlMcpServer,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, String> {
    let schema = args.get("schema").map(|s| s.as_str()).unwrap_or("dbo");
    let table = args
        .get("table")
        .ok_or("Missing required argument: table")?;

    // Get table columns
    let columns = server
        .metadata
        .get_table_columns(schema, table)
        .await
        .map_err(|e| e.to_string())?;

    if columns.is_empty() {
        return Err(format!("Table not found: {}.{}", schema, table));
    }

    // Filter out identity and computed columns
    let insertable_columns: Vec<_> = columns
        .iter()
        .filter(|c| !c.is_identity && !c.is_computed)
        .collect();

    let column_list = insertable_columns
        .iter()
        .map(|c| format!("[{}]", c.column_name))
        .collect::<Vec<_>>()
        .join(", ");

    let value_placeholders = insertable_columns
        .iter()
        .map(|c| {
            let placeholder = match c.data_type.to_uppercase().as_str() {
                "INT" | "BIGINT" | "SMALLINT" | "TINYINT" => "0".to_string(),
                "BIT" => "0".to_string(),
                "DECIMAL" | "NUMERIC" | "MONEY" | "SMALLMONEY" => "0.00".to_string(),
                "FLOAT" | "REAL" => "0.0".to_string(),
                "DATE" => "'YYYY-MM-DD'".to_string(),
                "TIME" => "'HH:MM:SS'".to_string(),
                "DATETIME" | "DATETIME2" | "SMALLDATETIME" => "'YYYY-MM-DD HH:MM:SS'".to_string(),
                "UNIQUEIDENTIFIER" => "NEWID()".to_string(),
                _ => format!("N'<{}>'", c.column_name),
            };
            format!("{} /* {} {} */", placeholder, c.column_name, c.data_type)
        })
        .collect::<Vec<_>>()
        .join(",\n    ");

    let column_desc = insertable_columns
        .iter()
        .map(|c| {
            format!(
                "- {} ({}){}",
                c.column_name,
                c.data_type,
                if !c.is_nullable { " - REQUIRED" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_text = format!(
        r#"Generate an INSERT statement for [{schema}].[{table}].

## Insertable Columns

{column_desc}

## Template

```sql
INSERT INTO [{schema}].[{table}] (
    {column_list}
)
VALUES (
    {value_placeholders}
);
```

## Instructions

Replace the placeholder values with actual data. Note:
- Columns marked REQUIRED cannot be NULL
- String values should use N'...' for Unicode support
- Date/time values should use ISO format
- UNIQUEIDENTIFIER can use NEWID() for auto-generation
"#
    );

    Ok(GetPromptResult {
        description: Some(format!("INSERT template for {}.{}", schema, table)),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}

async fn get_explain_procedure_prompt(
    server: &MssqlMcpServer,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, String> {
    let schema = args.get("schema").map(|s| s.as_str()).unwrap_or("dbo");
    let procedure = args
        .get("procedure")
        .ok_or("Missing required argument: procedure")?;

    // Get procedure definition
    let definition = server
        .metadata
        .get_procedure_definition(schema, procedure)
        .await
        .map_err(|e| e.to_string())?;

    // Get procedure parameters
    let parameters = server
        .metadata
        .get_procedure_parameters(schema, procedure)
        .await
        .map_err(|e| e.to_string())?;

    let param_desc = if parameters.is_empty() {
        "This procedure has no parameters.".to_string()
    } else {
        parameters
            .iter()
            .map(|p| {
                format!(
                    "- {} ({}){}{}",
                    p.parameter_name,
                    p.data_type,
                    if p.is_output { " OUTPUT" } else { "" },
                    if p.has_default { " [has default]" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let definition_text = definition.unwrap_or_else(|| "(Definition not available)".to_string());

    let prompt_text = format!(
        r#"Explain the stored procedure [{schema}].[{procedure}].

## Parameters

{param_desc}

## Definition

```sql
{definition_text}
```

## Please Explain

1. **Purpose**: What does this procedure do?
2. **Parameters**: Explain each parameter and its purpose
3. **Logic Flow**: Step-by-step explanation of what happens
4. **Return Values**: What does it return? (result sets, output parameters)
5. **Example Usage**: Show how to call this procedure
6. **Potential Issues**: Any edge cases or error conditions to watch for?
"#
    );

    Ok(GetPromptResult {
        description: Some(format!("Explanation of {}.{}", schema, procedure)),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}

fn get_optimize_query_prompt(args: &HashMap<String, String>) -> Result<GetPromptResult, String> {
    let query = args
        .get("query")
        .ok_or("Missing required argument: query")?;

    let prompt_text = format!(
        r#"Analyze and optimize the following SQL query.

## Original Query

```sql
{query}
```

## Analysis Requested

1. **Query Structure**
   - Is the query logically correct?
   - Any syntax issues or anti-patterns?

2. **Performance Issues**
   - Potential table scans
   - Missing indexes
   - Inefficient joins
   - Subquery vs JOIN considerations

3. **Optimizations**
   - Rewrite suggestions
   - Index recommendations
   - Query hints if appropriate

4. **Best Practices**
   - SET NOCOUNT ON for procedures
   - Avoiding SELECT *
   - Proper use of CTEs vs temp tables

5. **Optimized Version**
   - Provide the optimized query with comments
"#
    );

    Ok(GetPromptResult {
        description: Some("Query optimization analysis".to_string()),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}

fn get_debug_error_prompt(args: &HashMap<String, String>) -> Result<GetPromptResult, String> {
    let error = args
        .get("error")
        .ok_or("Missing required argument: error")?;
    let query = args.get("query");

    let mut prompt_text = format!(
        r#"Help debug this SQL Server error.

## Error Message

```
{error}
```
"#
    );

    if let Some(q) = query {
        prompt_text.push_str(&format!(
            r#"
## Query That Caused the Error

```sql
{q}
```
"#
        ));
    }

    prompt_text.push_str(
        r#"
## Please Provide

1. **Error Explanation**: What does this error mean?
2. **Common Causes**: Why does this error typically occur?
3. **Diagnosis Steps**: How to investigate further
4. **Solutions**: How to fix the issue
5. **Prevention**: How to avoid this error in the future
"#,
    );

    Ok(GetPromptResult {
        description: Some("SQL Server error debugging".to_string()),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt_text),
        }],
    })
}
