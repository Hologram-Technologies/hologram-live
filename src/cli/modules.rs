use super::{helpers, Cli};
use clap::{Args, Subcommand};
use hologram_live::error::Result;
use hologram_live::protocol::{RpcRequest, RpcResponse};

#[derive(Debug, Clone, Args)]
pub struct ModulesArgs {
    #[command(subcommand)]
    command: ModulesCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ModulesCommand {
    List,
    Graph,
}

pub async fn run(cli: Cli, args: ModulesArgs) -> Result<()> {
    let modules = match helpers::call(&cli, RpcRequest::ModulesList).await? {
        RpcResponse::Modules(value) => value,
        other => return helpers::unexpected(other),
    };
    match args.command {
        ModulesCommand::List => helpers::print(&cli, &modules),
        ModulesCommand::Graph => {
            let mut graph = Vec::new();
            for module in modules {
                if module.dependencies.is_empty() {
                    graph.push(module.id.clone());
                }
                for dependency in module.dependencies {
                    graph.push(format!("{dependency} -> {}", module.id));
                }
            }
            if cli.json {
                helpers::print(&cli, &graph)
            } else {
                for line in graph {
                    println!("{line}");
                }
                Ok(())
            }
        }
    }
}
