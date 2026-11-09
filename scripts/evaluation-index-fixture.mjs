// Evaluation substitute for indexing only. This deliberately does not test qmd
// search, embeddings or retrieval. Real model calls still go through the CLI.
if (process.argv.includes("vsearch")) process.stdout.write("[]\n");
