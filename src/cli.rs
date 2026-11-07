use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "vej", version, about = "High-performance, local System-1 AI decision engine competing with Jev")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the low-latency HTTP daemon with Jev-compatible API
    Serve(ServeArgs),

    /// Make a fast single-pass typed decision directly from CLI
    Decide(DecideArgs),

    /// Manage local models (list, download presets)
    Models(ModelsArgs),

    /// Manage configuration
    Config(ConfigArgs),
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Model to load (e.g. llama-3.2-1b, smollm2-1.7b, gemma-2-2b, or custom .gguf path)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Host interface to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Port to listen on
    #[arg(short, long, default_value_t = 8080)]
    pub port: u16,

    /// Extra browser origin allowed to call the API (repeatable), e.g. http://localhost:3000.
    /// The daemon's own origin is always allowed.
    #[arg(long = "allow-origin", value_name = "ORIGIN")]
    pub allow_origins: Vec<String>,

    /// Number of GPU layers to offload (default: 99 for full Metal/CUDA acceleration)
    #[arg(long)]
    pub gpu_layers: Option<u32>,

    /// Number of CPU threads
    #[arg(short, long)]
    pub threads: Option<u32>,
}

#[derive(Args, Debug)]
pub struct DecideArgs {
    /// Input state / text to classify (positional argument)
    #[arg(value_name = "STATE")]
    pub state_pos: Option<String>,

    /// Input state / text to classify (named option, e.g. --state "...")
    #[arg(short, long)]
    pub state: Option<String>,

    /// Comma-separated list of candidate choices (e.g. "refund,support,billing")
    #[arg(short, long, value_delimiter = ',')]
    pub choices: Vec<String>,

    /// Optional guidance instruction or classification task definition
    #[arg(short, long)]
    pub instruction: Option<String>,

    /// Model to use for the decision
    #[arg(short, long)]
    pub model: Option<String>,

    /// Softmax temperature
    #[arg(short, long, default_value_t = 1.0)]
    pub temperature: f32,

    /// Number of GPU layers to offload
    #[arg(long)]
    pub gpu_layers: Option<u32>,

    /// Number of CPU threads
    #[arg(long)]
    pub threads: Option<u32>,

    /// Output full JSON response
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCommands,
}

#[derive(Subcommand, Debug)]
pub enum ModelsCommands {
    /// List all supported models, availability, origin, and cache status
    List,

    /// Download a model preset from Hugging Face into the local cache
    Pull {
        /// Model name or alias (e.g. llama-3.2-1b, smollm2-1.7b, gemma-2-2b)
        model: String,
    },
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Initialize a default vej.toml in the current directory
    Init {
        /// Overwrite if existing
        #[arg(short, long)]
        force: bool,
    },
    /// Print current config path
    Path,
}
