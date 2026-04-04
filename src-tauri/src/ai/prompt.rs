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
        "format_sql" => {
            parts.push("The user will provide a SQL query. Reformat it with proper indentation, \
                         line breaks, and consistent casing for keywords. Return ONLY the formatted SQL, \
                         no explanation or markdown code fences.".to_string());
        }
        "comment_sql" => {
            parts.push("The user will provide a SQL query. Add clear, concise inline SQL comments \
                         (using -- syntax) explaining what each significant section does. \
                         Return ONLY the commented SQL, no additional explanation or markdown code fences.".to_string());
        }
        "generate_metadata" => {
            parts.push("The user will provide the structure of a single database object (table, view, function, or procedure). \
                         Generate documentation metadata as a JSON object with these exact fields:\n\
                         - \"description\": string, 1-3 sentence description of the object's purpose\n\
                         - \"columns\": array of {\"name\": string, \"description\": string} for each column or parameter\n\
                         - \"exampleUsage\": string, one practical SQL example using this object\n\
                         - \"relatedObjects\": array of strings, names of likely related tables/views inferred from naming, foreign keys, and conventions\n\
                         - \"dependencies\": array of strings, objects this depends on inferred from structure and naming\n\n\
                         Respond with ONLY the raw JSON object. No markdown code fences, no explanation, no extra text.".to_string());
        }
        _ => {}
    }

    parts.join("\n\n")
}
