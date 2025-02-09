mod cli;

use bundle::{self, index::Index, Bundle};
use clap::Parser;
use cli::Commands;
use cliclack::{MultiProgress, ProgressBar};
use fancy_duration::AsFancyDuration;
use globset::{Glob, GlobSetBuilder};
use human_repr::HumanCount;
use rayon::iter::ParallelIterator;
use std::{sync::Arc, time::Instant};
use tracing::{field::Visit, Subscriber};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

fn main() {
    let Commands {
        input,
        output,
        shaders,
        threads,
        filter,
        ..
    } = Commands::parse();

    let shaders = shaders.unwrap_or_default();

    let multiprogress = MultiProgress::new("Decompressing...");
    let progress = Arc::new(multiprogress.add(ProgressBar::new(0)));
    progress.start("Initializing..");

    let layer = CliClackProgressLayer::new(progress.clone());

    tracing_subscriber::registry().with(layer).init();
    // tracing_subscriber::fmt()
    //     .with_file(false)
    //     .with_file(false)
    //     .with_target(false)
    //     .with_timer(UtcTime::rfc_3339())
    //     .with_max_level(debug)
    //     .compact();

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads.unwrap_or_default().into())
        .build_global()
        .unwrap();

    assert!(input.exists());
    progress.set_message("Reading Index...");
    let file = std::fs::read(input.join("Bundles2").join("_.index.bin")).unwrap();

    let bundle: Bundle<Index> = bundle::Bundle::from_slice(file.as_slice()).unwrap();
    progress.set_message("Decompressing Index...");
    let index = bundle.decompress().unwrap();

    progress.set_message("Calculating total amount of file...");

    let mut builder = GlobSetBuilder::new();
    if let Some(ref filter) = filter {
        filter.split(',').for_each(|pat| {
            builder.add(Glob::new(pat).unwrap());
        });
    };
    let pattern = builder.build().unwrap();

    let iter = index
        .iter_bundles()
        .filter(|(bundle, _)| shaders || !bundle.path().contains("shadercache"))
        .filter_map(|(bundle, files)| {
            let matching: Arc<[_]> = files
                .iter()
                .filter(|(path, _)| pattern.is_match(path.to_str().unwrap()))
                .cloned()
                .collect::<Vec<_>>()
                .into();

            if !matching.is_empty() {
                Some((bundle, matching))
            } else {
                None
            }
        });

    let total: u64 = iter
        .clone()
        .map(|(_, files)| {
            files
                .iter()
                .filter(|(path, _)| {
                    shaders
                        || !path
                            .components()
                            .filter_map(|c| c.as_os_str().to_str())
                            .any(|c| c.contains("shadercache"))
                })
                .count() as u64
        })
        .sum();

    progress.set_length(total);
    progress.set_message("Starting...");
    let start = Instant::now();

    let bytes = index.extract(iter, input, output, shaders);

    progress.stop(format!(
        "Extracted in {} | Bytes Written: {}",
        start.elapsed().fancy_duration(),
        bytes.human_count_bytes()
    ));
    multiprogress.stop();
}

struct CliClackProgressLayer {
    progress: Arc<ProgressBar>,
}

impl CliClackProgressLayer {
    fn new(progress: Arc<ProgressBar>) -> Self {
        Self { progress }
    }
}

impl<S> Layer<S> for CliClackProgressLayer
where
    S: Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = CliClackVisitor(&self.progress);
        event.record(&mut visitor);
    }
}

struct CliClackVisitor<'a>(&'a ProgressBar);

impl Visit for CliClackVisitor<'_> {
    fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {}
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "bundle" {
            let width = value.len().min(50);
            self.0.set_message(format!("{:<50}...", &value[..width]))
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "done" {
            self.0.inc(value)
        }
    }
}
