#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code, non_snake_case, unused_parens, unused_variables)]
extern crate alloc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeError {
    AddOverflow,
    MulOverflow,
    ShiftExponentTooLarge,
    ShiftOverflow,
    PowExponentTooLarge,
    PowOverflow,
    OutputTooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CliCommand {
    Run = 0,
    Serve = 1,
    Pull = 2,
    Push = 3,
    Inspect = 4,
    Chat = 5,
    Nodes = 6,
    Status = 7,
    Stop = 8,
    Restart = 9,
    Update = 10,
    Init = 11,
    Compile = 12,
    Files = 13,
    Doctor = 14,
    Ai = 15,
    Plugins = 16,
    Oci = 17,
    Models = 18,
    Route = 19,
    App = 20,
    Config = 21,
    Tracing = 22,
    History = 23,
    Openapi = 24,
    Verify = 25,
    Plan = 26,
    Help = 27,
    Unknown = 28,
}

pub fn executeCommand(cmd: crate::CliCommand) -> alloc::string::String {
    match cmd {
        crate::CliCommand::Run => alloc::string::String::from("{\"command\":\"run\",\"status\":\"ok\"}"),
        crate::CliCommand::Serve => alloc::string::String::from("{\"command\":\"serve\",\"status\":\"ok\"}"),
        crate::CliCommand::Pull => alloc::string::String::from("{\"command\":\"pull\",\"status\":\"ok\"}"),
        crate::CliCommand::Push => alloc::string::String::from("{\"command\":\"push\",\"status\":\"ok\"}"),
        crate::CliCommand::Inspect => alloc::string::String::from("{\"holo_version\":4,\"magic\":\"HOLO\"}"),
        crate::CliCommand::Chat => alloc::string::String::from("{\"command\":\"chat\",\"status\":\"ok\"}"),
        crate::CliCommand::Nodes => alloc::string::String::from("{\"command\":\"nodes\",\"status\":\"ok\"}"),
        crate::CliCommand::Status => alloc::string::String::from("{\"status\":\"ready\",\"version\":\"1.0.0\",\"engine\":\"prismpm\"}"),
        crate::CliCommand::Stop => alloc::string::String::from("{\"command\":\"stop\",\"status\":\"ok\"}"),
        crate::CliCommand::Restart => alloc::string::String::from("{\"command\":\"restart\",\"status\":\"ok\"}"),
        crate::CliCommand::Update => alloc::string::String::from("{\"command\":\"update\",\"status\":\"ok\"}"),
        crate::CliCommand::Init => alloc::string::String::from("{\"command\":\"init\",\"status\":\"ok\"}"),
        crate::CliCommand::Compile => alloc::string::String::from("{\"command\":\"compile\",\"status\":\"ok\"}"),
        crate::CliCommand::Files => alloc::string::String::from("{\"command\":\"files\",\"status\":\"ok\"}"),
        crate::CliCommand::Doctor => alloc::string::String::from("{\"command\":\"doctor\",\"status\":\"ok\"}"),
        crate::CliCommand::Ai => alloc::string::String::from("{\"service\":\"hologram-ai\",\"cost_model\":\"uor-prism\",\"status\":\"optimal\"}"),
        crate::CliCommand::Plugins => alloc::string::String::from("{\"command\":\"plugins\",\"status\":\"ok\"}"),
        crate::CliCommand::Oci => alloc::string::String::from("{\"command\":\"oci\",\"status\":\"ok\"}"),
        crate::CliCommand::Models => alloc::string::String::from("{\"command\":\"models\",\"status\":\"ok\"}"),
        crate::CliCommand::Route => alloc::string::String::from("{\"command\":\"route\",\"status\":\"ok\"}"),
        crate::CliCommand::App => alloc::string::String::from("{\"command\":\"app\",\"status\":\"ok\"}"),
        crate::CliCommand::Config => alloc::string::String::from("{\"command\":\"config\",\"status\":\"ok\"}"),
        crate::CliCommand::Tracing => alloc::string::String::from("{\"command\":\"tracing\",\"status\":\"ok\"}"),
        crate::CliCommand::History => alloc::string::String::from("{\"command\":\"history\",\"status\":\"ok\"}"),
        crate::CliCommand::Openapi => alloc::string::String::from("{\"command\":\"openapi\",\"status\":\"ok\"}"),
        crate::CliCommand::Verify => alloc::string::String::from("{\"command\":\"verify\",\"status\":\"ok\"}"),
        crate::CliCommand::Plan => alloc::string::String::from("{\"command\":\"plan\",\"status\":\"ok\"}"),
        crate::CliCommand::Help => alloc::string::String::from("{\"help\":\"hologram <command>\",\"commands\":28}"),
        crate::CliCommand::Unknown => alloc::string::String::from("{\"error\":\"unknown command\"}"),
    }
}

pub fn parseCliCommand(name: alloc::string::String) -> crate::CliCommand {
    __prod_borrowed_parseCliCommand(name.as_ref())
}

fn __prod_borrowed_parseCliCommand(name: &str) -> crate::CliCommand {
    { let _x_4 = name == "run"; match _x_4 {
        false => { let _x_5511 = name == "serve"; match _x_5511 {
        false => { let _x_5619 = name == "pull"; match _x_5619 {
        false => { let _x_5723 = name == "push"; match _x_5723 {
        false => { let _x_5823 = name == "inspect"; match _x_5823 {
        false => { let _x_5919 = name == "chat"; match _x_5919 {
        false => { let _x_6011 = name == "nodes"; match _x_6011 {
        false => { let _x_6099 = name == "status"; match _x_6099 {
        false => { let _x_6183 = name == "stop"; match _x_6183 {
        false => { let _x_6263 = name == "restart"; match _x_6263 {
        false => { let _x_6339 = name == "update"; match _x_6339 {
        false => { let _x_6411 = name == "init"; match _x_6411 {
        false => { let _x_6479 = name == "compile"; match _x_6479 {
        false => { let _x_6543 = name == "files"; match _x_6543 {
        false => { let _x_6603 = name == "doctor"; match _x_6603 {
        false => { let _x_6659 = name == "ai"; match _x_6659 {
        false => { let _x_6711 = name == "plugins"; match _x_6711 {
        false => { let _x_6759 = name == "oci"; match _x_6759 {
        false => { let _x_6803 = name == "models"; match _x_6803 {
        false => { let _x_6843 = name == "route"; match _x_6843 {
        false => { let _x_6879 = name == "app"; match _x_6879 {
        false => { let _x_6911 = name == "config"; match _x_6911 {
        false => { let _x_6939 = name == "tracing"; match _x_6939 {
        false => { let _x_6963 = name == "history"; match _x_6963 {
        false => { let _x_6983 = name == "openapi"; match _x_6983 {
        false => { let _x_6999 = name == "verify"; match _x_6999 {
        false => { let _x_7011 = name == "plan"; match _x_7011 {
        false => { let _x_7019 = name == "help"; match _x_7019 {
        false => {  crate::CliCommand::Unknown },
        true => {  crate::CliCommand::Help },
    } },
        true => {  crate::CliCommand::Plan },
    } },
        true => {  crate::CliCommand::Verify },
    } },
        true => {  crate::CliCommand::Openapi },
    } },
        true => {  crate::CliCommand::History },
    } },
        true => {  crate::CliCommand::Tracing },
    } },
        true => {  crate::CliCommand::Config },
    } },
        true => {  crate::CliCommand::App },
    } },
        true => {  crate::CliCommand::Route },
    } },
        true => {  crate::CliCommand::Models },
    } },
        true => {  crate::CliCommand::Oci },
    } },
        true => {  crate::CliCommand::Plugins },
    } },
        true => {  crate::CliCommand::Ai },
    } },
        true => {  crate::CliCommand::Doctor },
    } },
        true => {  crate::CliCommand::Files },
    } },
        true => {  crate::CliCommand::Compile },
    } },
        true => {  crate::CliCommand::Init },
    } },
        true => {  crate::CliCommand::Update },
    } },
        true => {  crate::CliCommand::Restart },
    } },
        true => {  crate::CliCommand::Stop },
    } },
        true => {  crate::CliCommand::Status },
    } },
        true => {  crate::CliCommand::Nodes },
    } },
        true => {  crate::CliCommand::Chat },
    } },
        true => {  crate::CliCommand::Inspect },
    } },
        true => {  crate::CliCommand::Push },
    } },
        true => {  crate::CliCommand::Pull },
    } },
        true => {  crate::CliCommand::Serve },
    } },
        true => {  crate::CliCommand::Run },
    } }
}

pub fn dispatchBytes(value: alloc::vec::Vec<u8>) -> alloc::vec::Vec<u8> {
    { let _x_9 = alloc::string::String::from_utf8(value).ok(); match _x_9 {
        None => {  (alloc::string::String::from("{\"error\":\"malformed-utf8\"}")).into_bytes() },
        Some(val_12) => { let _x_19 = dispatchString(val_12); (_x_19).into_bytes() },
    } }
}

pub fn dispatchString(value: alloc::string::String) -> alloc::string::String {
    { let _x_1 = __prod_borrowed_parseCliCommand((value).as_ref()); executeCommand(_x_1) }
}
