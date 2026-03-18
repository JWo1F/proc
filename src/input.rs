/// A parsed interactive command.
#[derive(Debug, PartialEq)]
pub enum Command {
    Kill(String),
    Restart(String),
    Add(String, String),
    Remove(String),
    List,
    Help,
}

/// Events sent from the input module to the manager.
pub enum InputEvent {
    Command(Command),
    CtrlC,
}

/// Parse a user input line into a Command.
pub fn parse_command(input: &str) -> Result<Command, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(String::new());
    }

    let (cmd, rest) = match input.split_once(' ') {
        Some((c, r)) => (c, r.trim()),
        None => (input, ""),
    };

    match cmd {
        "kill" => {
            if rest.is_empty() {
                return Err("Usage: kill <name>".to_string());
            }
            Ok(Command::Kill(rest.to_string()))
        }
        "restart" => {
            if rest.is_empty() {
                return Err("Usage: restart <name>".to_string());
            }
            Ok(Command::Restart(rest.to_string()))
        }
        "add" => {
            let Some((name, cmd)) = rest.split_once(':') else {
                return Err("Usage: add <name>: <command>".to_string());
            };
            let name = name.trim();
            let cmd = cmd.trim();
            if name.is_empty() || cmd.is_empty() {
                return Err("Usage: add <name>: <command>".to_string());
            }
            Ok(Command::Add(name.to_string(), cmd.to_string()))
        }
        "remove" => {
            if rest.is_empty() {
                return Err("Usage: remove <name>".to_string());
            }
            Ok(Command::Remove(rest.to_string()))
        }
        "list" | "ps" => Ok(Command::List),
        "help" => Ok(Command::Help),
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kill() {
        assert_eq!(parse_command("kill web"), Ok(Command::Kill("web".to_string())));
    }

    #[test]
    fn parse_restart() {
        assert_eq!(parse_command("restart web"), Ok(Command::Restart("web".to_string())));
    }

    #[test]
    fn parse_add_with_colon() {
        assert_eq!(
            parse_command("add worker: bundle exec sidekiq"),
            Ok(Command::Add("worker".to_string(), "bundle exec sidekiq".to_string()))
        );
    }

    #[test]
    fn parse_add_missing_colon() {
        assert!(parse_command("add worker").is_err());
    }

    #[test]
    fn parse_remove() {
        assert_eq!(parse_command("remove web"), Ok(Command::Remove("web".to_string())));
    }

    #[test]
    fn parse_list() {
        assert_eq!(parse_command("list"), Ok(Command::List));
    }

    #[test]
    fn parse_ps_alias() {
        assert_eq!(parse_command("ps"), Ok(Command::List));
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse_command("help"), Ok(Command::Help));
    }

    #[test]
    fn parse_unknown_command() {
        assert!(parse_command("foo").is_err());
    }

    #[test]
    fn parse_empty_input() {
        assert!(parse_command("").is_err());
    }

    #[test]
    fn parse_whitespace_only() {
        assert!(parse_command("   ").is_err());
    }

    #[test]
    fn parse_kill_missing_name() {
        assert!(parse_command("kill").is_err());
    }
}
