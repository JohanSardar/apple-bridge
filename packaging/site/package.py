import argparse
import shutil
from pathlib import Path

parser = argparse.ArgumentParser(description="Stage the public Eilmer website and canonical installer into a new directory.")
parser.add_argument("output", type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
shutil.copytree(root / "deploy/site", args.output, ignore=shutil.ignore_patterns("README.md", "serve.py", "Caddyfile", "eilmer-site.service", "__pycache__"))
shutil.copyfile(root / "docs/install", args.output / "applebridge/install")
shutil.copytree(root / "docs", args.output / "applebridge/docs", ignore=shutil.ignore_patterns("install"))
shutil.copyfile(root / "rust/crates/ab-runtime/skills/setup.md", args.output / "applebridge/setup.md")
print(f"Staged website in {args.output}")
