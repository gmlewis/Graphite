#!/usr/bin/env python3
"""Parse #[node_macro::node] annotated Rust functions and produce node_catalog.json."""

import json
import os
import re
import sys
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Optional


REPO_ROOT = Path(__file__).resolve().parent.parent
NODE_GRAPH_DIR = REPO_ROOT / "node-graph" / "nodes"


@dataclass
class FieldInfo:
    name: str
    description: str = ""
    rust_type: str = ""
    hidden: bool = False
    exposed: bool = False
    default: Optional[str] = None
    scope: Optional[str] = None
    implementations: list[str] = field(default_factory=list)
    widget: Optional[str] = None
    soft_min: Optional[float] = None
    soft_max: Optional[float] = None
    hard_min: Optional[float] = None
    hard_max: Optional[float] = None
    range: bool = False
    unit: Optional[str] = None
    step: Optional[float] = None
    display_decimal_places: Optional[int] = None
    gpu_image: bool = False
    data: bool = False


@dataclass
class NodeDef:
    name: str
    node_id: str
    category: str
    description: str = ""
    file: str = ""
    line: int = 0
    inputs: list[FieldInfo] = field(default_factory=list)
    output_type: str = ""
    properties: Optional[str] = None
    cfg: Optional[str] = None
    shader_node: Optional[str] = None
    memoize: bool = False
    inject_scope: bool = False
    path: Optional[str] = None
    async_fn: bool = False


def find_rs_files(root: Path) -> list[Path]:
    files = []
    for dirpath, _, filenames in os.walk(root):
        for f in filenames:
            if f.endswith(".rs"):
                files.append(Path(dirpath) / f)
    return sorted(files)


def extract_node_blocks(source: str, filepath: str) -> list[NodeDef]:
    nodes = []
    lines = source.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        # Look for #[node_macro::node( ... )]
        if "#[node_macro::node(" in line:
            attr_start = i
            # Collect full attribute (may span multiple lines)
            attr_lines = [line]
            paren_depth = line.count("(") - line.count(")")
            while paren_depth > 0 and i + 1 < len(lines):
                i += 1
                attr_lines.append(lines[i])
                paren_depth += lines[i].count("(") - lines[i].count(")")

            attr_text = "\n".join(attr_lines)

            # Parse attribute parameters
            attr_params = parse_node_attr(attr_text)

            # Collect doc comments above the attribute
            doc_lines = []
            j = attr_start - 1
            while j >= 0 and lines[j].strip().startswith("///"):
                doc_lines.insert(0, lines[j].strip().lstrip("/ ").strip())
                j -= 1
            description = "\n".join(doc_lines).strip()

            # Find the function signature (next fn keyword)
            i += 1
            fn_start = i
            while i < len(lines) and "fn " not in lines[i]:
                i += 1
            if i >= len(lines):
                break

            # Collect full function signature (may span multiple lines)
            fn_lines = [lines[i]]
            brace_depth = lines[i].count("{") - lines[i].count("}")
            while brace_depth <= 0 and i + 1 < len(lines):
                i += 1
                fn_lines.append(lines[i])
                brace_depth += lines[i].count("{") - lines[i].count("}")

            fn_text = "\n".join(fn_lines)

            # Check for async
            is_async = "async fn " in fn_text

            # Parse function name
            fn_match = re.search(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", fn_text)
            if not fn_match:
                i += 1
                continue
            fn_name = fn_match.group(1)

            # Parse display name: use #[name("...")] if present, else convert fn_name
            display_name = attr_params.get("name", fn_name.replace("_", " ").title())

            # Parse function parameters
            inputs = parse_fn_params(fn_text)

            # Parse return type
            output_type = parse_return_type(fn_text)

            node = NodeDef(
                name=display_name,
                node_id=fn_name,
                category=attr_params.get("category", ""),
                description=description,
                file=str(filepath),
                line=attr_start + 1,
                inputs=inputs,
                output_type=output_type,
                properties=attr_params.get("properties"),
                cfg=attr_params.get("cfg"),
                shader_node=attr_params.get("shader_node"),
                memoize=attr_params.get("memoize", "false") == "true",
                inject_scope=attr_params.get("inject_scope", "false") == "true",
                path=attr_params.get("path"),
                async_fn=is_async,
            )
            nodes.append(node)
        i += 1
    return nodes


def parse_node_attr(attr_text: str) -> dict:
    """Parse the contents of #[node_macro::node(...)] to extract key-value params."""
    params = {}
    # Remove the wrapper
    inner = re.sub(r"#\[node_macro::node\((.*)\)\]", r"\1", attr_text, flags=re.DOTALL)
    # Parse named params like category("..."), name("..."), properties("..."), etc.
    for m in re.finditer(r'(\w+)\("([^"]*)"\)', inner):
        params[m.group(1)] = m.group(2)
    # Parse boolean params like memoize(true), inject_scope(true)
    for m in re.finditer(r'(\w+)\((true|false)\)', inner):
        params[m.group(1)] = m.group(2)
    return params


def parse_fn_params(fn_text: str) -> list[FieldInfo]:
    """Parse function parameters from the function signature, including attributes and doc comments."""
    # Extract the parameter list between the first ( and matching )
    paren_start = fn_text.index("(")
    depth = 0
    paren_end = paren_start
    for ci, ch in enumerate(fn_text[paren_start:], paren_start):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                paren_end = ci
                break

    param_block = fn_text[paren_start + 1 : paren_end]

    # Parse line-by-line to correctly handle /// doc comments
    lines = param_block.split("\n")
    fields = []
    current_doc_lines = []
    current_attr_lines = []

    for line in lines:
        stripped = line.strip()
        if not stripped:
            continue

        if stripped.startswith("///"):
            # Doc comment line - collect it
            current_doc_lines.append(stripped.lstrip("/ ").strip())
            continue

        if stripped.startswith("#["):
            # Attribute line - collect it
            current_attr_lines.append(stripped)
            continue

        # This should be a parameter line like "name: Type," or "name: Type"
        # It could also be the ctx parameter which we skip
        m = re.match(r"(?:pub\s+)?(?:mut\s+)?(\w+)\s*:", stripped)
        if not m:
            continue

        param_name = m.group(1)

        # Skip the first param (ctx)
        if re.match(r"_?\s*:\s*impl\s+Ctx", stripped) or re.match(r"ctx\s*:", stripped):
            current_doc_lines = []
            current_attr_lines = []
            continue

        # Build a chunk with doc comments and attributes prepended
        chunk_lines = []
        for doc in current_doc_lines:
            chunk_lines.append(f"/// {doc}")
        for attr in current_attr_lines:
            chunk_lines.append(attr)
        chunk_lines.append(stripped)

        chunk = "\n".join(chunk_lines)
        field = parse_param_chunk(chunk)
        if field:
            fields.append(field)

        current_doc_lines = []
        current_attr_lines = []

    return fields


def split_params(block: str) -> list[str]:
    """Split a parameter block on top-level commas, respecting nested brackets."""
    chunks = []
    depth = 0
    current = []
    angle_depth = 0
    for ch in block:
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == "<":
            angle_depth += 1
        elif ch == ">":
            angle_depth -= 1

        if ch == "," and depth == 0 and angle_depth == 0:
            chunks.append("".join(current))
            current = []
        else:
            current.append(ch)
    if current:
        chunks.append("".join(current))
    return chunks


def parse_param_chunk(chunk: str) -> Optional[FieldInfo]:
    """Parse a single parameter chunk including any preceding attributes and doc comments."""
    lines = [l.strip() for l in chunk.strip().split("\n") if l.strip()]

    field = FieldInfo(name="")
    param_line = ""

    for line in lines:
        # Doc comment lines
        if line.startswith("///"):
            doc_text = line.lstrip("/ ").strip()
            if doc_text:
                if field.description:
                    field.description += "\n" + doc_text
                else:
                    field.description = doc_text
            continue

        # Attribute lines
        attr_match = re.match(r"#\[(.+)\]", line)
        if attr_match:
            attr_text = attr_match.group(1)
            # Classify the attribute
            if attr_text.startswith("default("):
                val = extract_paren_content(attr_text, "default")
                field.default = val
            elif attr_text.startswith("implementations("):
                impls = extract_paren_content(attr_text, "implementations")
                field.implementations = [i.strip() for i in impls.split(",")]
            elif attr_text.startswith("widget("):
                field.widget = extract_paren_content(attr_text, "widget")
            elif attr_text.startswith("soft("):
                val = extract_paren_content(attr_text, "soft")
                rng = parse_range(val)
                if rng:
                    field.soft_min, field.soft_max = rng
            elif attr_text.startswith("hard("):
                val = extract_paren_content(attr_text, "hard")
                rng = parse_range(val)
                if rng:
                    field.hard_min, field.hard_max = rng
            elif attr_text == "range":
                field.range = True
            elif attr_text.startswith("unit("):
                field.unit = extract_paren_content(attr_text, "unit")
            elif attr_text.startswith("step("):
                val = extract_paren_content(attr_text, "step")
                try:
                    field.step = float(val)
                except ValueError:
                    pass
            elif attr_text.startswith("display_decimal_places("):
                val = extract_paren_content(attr_text, "display_decimal_places")
                try:
                    field.display_decimal_places = int(val)
                except ValueError:
                    pass
            elif attr_text == "gpu_image":
                field.gpu_image = True
            elif attr_text == "data":
                field.data = True
            elif attr_text == "expose":
                field.exposed = True
            elif attr_text.startswith("scope("):
                field.scope = extract_paren_content(attr_text, "scope")
            elif attr_text.startswith("name("):
                pass  # handled at node level
        else:
            param_line = line

    if not param_line:
        return None

    # Parse "name: Type" or "name: Type = default"
    # Handle patterns like:
    #   #[default(true)] fill: bool,
    #   mut input: T,
    #   _primary: (),
    #   #[implementations(f64, f32)] operand_a: T,
    m = re.match(r"(?:pub\s+)?(?:mut\s+)?(\w+)\s*:\s*(.+?)(?:\s*,\s*$|\s*$)", param_line)
    if not m:
        return None

    name = m.group(1)
    rust_type = m.group(2).strip().rstrip(",").strip()

    # Check for doc comment in the param (usually on a preceding line)
    # We handle description elsewhere by looking at /// comments

    # Determine hidden status
    field.name = name
    field.rust_type = rust_type
    field.hidden = name.startswith("_") and name != "_primary"

    return field


def extract_paren_content(attr_text: str, attr_name: str) -> str:
    """Extract content inside the outermost parentheses of an attribute like default(...)"""
    pattern = rf"{attr_name}\((.*)\)"
    m = re.search(pattern, attr_text, re.DOTALL)
    if m:
        return m.group(1).strip()
    return ""


def parse_range(val: str) -> Optional[tuple[float, float]]:
    """Parse 'min..max' or 'min..' or '..max'."""
    m = re.match(r"([\d.]+)\.\.([\d.]+)", val)
    if m:
        return (float(m.group(1)), float(m.group(2)))
    m = re.match(r"([\d.]+)\.\.", val)
    if m:
        return (float(m.group(1)), None)
    m = re.match(r"\.\.([\d.]+)", val)
    if m:
        return (None, float(m.group(1)))
    return None


def parse_return_type(fn_text: str) -> str:
    """Extract the return type from a function signature."""
    # Find the -> before the opening {
    m = re.search(r"\)\s*->\s*(.+?)\s*\{", fn_text)
    if m:
        return m.group(1).strip()
    return ""


def main():
    output_path = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO_ROOT / "node_catalog.json"

    rs_files = find_rs_files(NODE_GRAPH_DIR)
    print(f"Scanning {len(rs_files)} Rust files in {NODE_GRAPH_DIR}")

    all_nodes = []
    for filepath in rs_files:
        source = filepath.read_text(errors="replace")
        nodes = extract_node_blocks(source, str(filepath.relative_to(REPO_ROOT)))
        if nodes:
            print(f"  {filepath.relative_to(REPO_ROOT)}: {len(nodes)} nodes")
        all_nodes.extend(nodes)

    # Sort by category then name
    all_nodes.sort(key=lambda n: (n.category, n.name))

    # Group by category
    categories = {}
    for node in all_nodes:
        cat = node.category or "(hidden)"
        categories.setdefault(cat, []).append(node)

    catalog = {
        "version": 1,
        "total_nodes": len(all_nodes),
        "categories": {},
    }

    for cat, nodes in sorted(categories.items()):
        catalog["categories"][cat] = {
            "count": len(nodes),
            "nodes": [asdict(n) for n in nodes],
        }

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(catalog, f, indent=2, default=str)

    print(f"\nWrote {len(all_nodes)} nodes across {len(categories)} categories to {output_path}")


if __name__ == "__main__":
    main()
