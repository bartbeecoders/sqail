/// Build the system prompt for an AI flow.
pub fn build_system_prompt(flow: &str, driver: Option<&str>, schema_context: Option<&str>) -> String {
    let mut parts = Vec::new();

    parts.push("You are a SQL assistant integrated into a desktop database editor called SQaiL. \
                 You help users write, understand, and optimize SQL queries.".to_string());

    if let Some(d) = driver {
        parts.push(format!("The target database dialect is: {d}. Write SQL compatible with this dialect."));
    }

    if let Some(ctx) = schema_context {
        if !ctx.is_empty() {
            parts.push(format!("Here is the database schema context:\n\n{ctx}"));
        }
    }

    match flow {
        "generate_sql" => {
            parts.push("The user will describe what they want in natural language. \
                         Respond with ONLY the SQL query, no explanation or markdown code fences. \
                         Use the provided schema context to write accurate queries with correct table and column names.".to_string());
        }
        "explain" => {
            parts.push("The user will provide a SQL query. Explain what it does in clear, \
                         concise plain English. Break down complex queries into logical steps.".to_string());
        }
        "optimize" => {
            parts.push("The user will provide a SQL query. Suggest an optimized version with explanations \
                         of what you changed and why. Consider indexing opportunities, query structure, \
                         and dialect-specific optimizations. Return the optimized SQL first, then the explanation.".to_string());
        }
        "document" => {
            parts.push("The user will provide database schema information. Generate clear markdown \
                         documentation describing the tables, their columns, relationships, and purpose. \
                         Infer likely purpose from naming conventions.".to_string());
        }
        _ => {}
    }

    parts.join("\n\n")
}
