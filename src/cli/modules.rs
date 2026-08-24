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
            for module in modules {
                if module.dependencies.is_empty() {
                    println!("{}", module.id);
                }
                for dependency in module.dependencies {
                    println!("{dependency} -> {}", module.id);
                }
            }
            Ok(())
        }
    }
}
