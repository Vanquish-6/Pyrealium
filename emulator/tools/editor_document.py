"""Bounded, conflict-checked offline editor transactions and reversible history."""
from collections import Counter
from pathlib import Path
from types import SimpleNamespace
import math
import os
import tempfile

from place_content import bounded_text, place


def read_document(path):
    return bounded_text(Path(path)) if Path(path).exists() else None


def rows_from_text(text):
    lines = (text if text is not None else "pyrealium-world-v2\n").splitlines()
    if not lines or lines[0] not in ("pyrealium-world-v2", "pyrealium-world-v3"):
        raise ValueError("Choose a world-v2 or world-v3 content file")
    size = 11 if lines[0].endswith("2") else 12
    rows = []
    identities = set()
    cells = Counter()
    for number, line in enumerate(lines[1:], 2):
        if not line or line.startswith("#"):
            continue
        try:
            row = line.split("\t")
            if len(row) != size or len(line.encode("utf-8")) > 512:
                raise ValueError("invalid placement shape")
            identity, entity, visual, cell = (int(value, 16) for value in row[:4])
            if identity in identities or identity >> 48 != 0x8140 or identity == 0x8140000000000000:
                raise ValueError("duplicate or invalid object identity")
            if entity >> 24 != 0x47 or visual >> 24 != 0x1f:
                raise ValueError("invalid asset identifier")
            low = cell & 65535
            if not 0 < cell <= 0xffffffff or not (1 <= low <= 64 or 256 <= low <= 65531):
                raise ValueError("invalid cell identifier")
            if any(not math.isfinite(float(v)) or abs(float(v)) >= 1_000_000 for v in row[4:7]):
                raise ValueError("invalid position")
            q = [float(v) for v in row[7:11]]
            if not all(math.isfinite(v) for v in q) or abs(sum(v*v for v in q)-1) > .01:
                raise ValueError("invalid orientation")
            if size == 12 and row[11] != "-":
                if not math.isfinite(float(row[11])) or not 0 < float(row[11]) <= 100:
                    raise ValueError("invalid scale")
            identities.add(identity)
            cells[cell] += 1
            rows.append(row)
        except (ValueError, OverflowError) as error:
            raise ValueError(f"Line {number}: {error}") from error
    if len(rows) > 16384 or any(count > 64 for count in cells.values()):
        raise ValueError("World/cell content limit exceeded")
    return rows


def atomic_write(path, text):
    descriptor, name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as file:
            file.write(text)
            file.flush()
            os.fsync(file.fileno())
        Path(name).replace(path)
    finally:
        Path(name).unlink(missing_ok=True)


def commit_document(path, expected, result):
    """Same lock as the CLI writer; keep a persistent pre-save backup."""
    path = Path(path)
    rows_from_text(result)
    if len(result.encode("utf-8")) > 4*1024*1024:
        raise ValueError("Content exceeds 4 MiB")
    path.parent.mkdir(parents=True, exist_ok=True)
    lock = path.with_name(path.name + ".edit.lock")
    with lock.open("x", encoding="utf-8") as guard:
        try:
            guard.write(str(os.getpid()))
            guard.flush()
            if read_document(path) != expected:
                raise ValueError("This file changed outside the editor. Reload it before saving.")
            if expected is not None:
                atomic_write(path.with_name(path.name + ".bak"), expected)
            atomic_write(path, result)
        finally:
            guard.close()
            lock.unlink()


def prepare_change(args, original, identity=None, delete=False, preserve_orientation=False):
    """Validate additions/updates through the existing native DAT authoring path."""
    rows = rows_from_text(original)
    target = next((row for row in rows if int(row[0], 16) == identity), None)
    if identity is not None and target is None:
        raise ValueError("Selected object no longer exists")
    lines = (original if original is not None else "pyrealium-world-v2\n").splitlines()
    target_index = next((i for i, line in enumerate(lines) if target and line == "\t".join(target)), None)
    if target_index is not None:
        lines.pop(target_index)
    if delete:
        if target is None:
            raise ValueError("Select a saved placement to delete")
        return "\n".join(lines) + "\n", identity
    with tempfile.TemporaryDirectory(prefix="pyrealium-editor-") as folder:
        staged = Path(folder) / "world.tsv"
        staged.write_text("\n".join(lines) + "\n", encoding="utf-8")
        staged_args = SimpleNamespace(**vars(args))
        staged_args.output = staged
        new_id = place(staged_args)
        result = bounded_text(staged)
    if target is None:
        return result, new_id
    new_row = rows_from_text(result)[-1]
    new_row[0] = target[0]
    if preserve_orientation:
        new_row[7:11] = target[7:11]
    # Keep the original ordering and comments; only change this object's row.
    result_lines = result.splitlines()
    header = result_lines[0]
    original_lines = original.splitlines()
    if header != original_lines[0]:
        original_lines = [header] + [
            line + "\t-" if line and not line.startswith("#") else line
            for line in original_lines[1:]]
    original_lines[target_index] = "\t".join(new_row)
    return "\n".join(original_lines) + "\n", identity


def heading_from_row(row):
    return round((math.atan2(-float(row[10]), float(row[7])) * 255 / math.pi) % 255)
