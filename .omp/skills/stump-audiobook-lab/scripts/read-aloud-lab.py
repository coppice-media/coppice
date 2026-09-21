#!/usr/bin/env python3
"""Deterministic, fail-closed helpers for private Stump read-aloud fixtures.

The CLI deliberately keeps the source EPUB/M4B untouched.  It shells out only
for media probing/copying (ffprobe/ffmpeg) and uses the Python standard library
for ZIP, XML, hashing, and JSON work.  It never embeds source text or audio.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import posixpath
import re
import shutil
import subprocess
import sys
import tempfile
import unicodedata
import zipfile
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from pathlib import Path
from typing import Any, Iterable, Optional
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET


LAB_VERSION = "1"
MIMETYPE = "application/epub+zip"
NS_OPF = "http://www.idpf.org/2007/opf"
NS_DCTERMS = "http://purl.org/dc/terms/"
NS_DC = "http://purl.org/dc/elements/1.1/"
NS_EPUB = "http://www.idpf.org/2007/ops"
NS_XML = "http://www.w3.org/XML/1998/namespace"


class LabError(RuntimeError):
    """An operator-fixable, intentionally user-facing failure."""


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def attr(element: ET.Element, name: str, namespace: Optional[str] = None) -> Optional[str]:
    if namespace is not None:
        return element.attrib.get("{%s}%s" % (namespace, name))
    if name in element.attrib:
        return element.attrib[name]
    for key, value in element.attrib.items():
        if local_name(key) == name:
            return value
    return None


def tokens(value: Optional[str]) -> set[str]:
    return {part for part in (value or "").split() if part}


def text_content(element: ET.Element) -> str:
    return " ".join(part.strip() for part in element.itertext() if part and part.strip())


def emit_json(value: Any) -> None:
    sys.stdout.write(json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        raise LabError("cannot read %s: %s" % (path, exc)) from exc
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_xml(data: bytes, label: str) -> ET.Element:
    try:
        return ET.fromstring(data)
    except ET.ParseError as exc:
        raise LabError("invalid XML in %s: %s" % (label, exc)) from exc


def finite_decimal(value: Any, label: str) -> Decimal:
    try:
        number = Decimal(str(value))
    except (InvalidOperation, ValueError) as exc:
        raise LabError("%s is not a finite number: %r" % (label, value)) from exc
    if not number.is_finite():
        raise LabError("%s is not finite: %r" % (label, value))
    return number


def decimal_ms(value: Any, label: str) -> int:
    number = finite_decimal(value, label) * Decimal("1000")
    return int(number.quantize(Decimal("1"), rounding=ROUND_HALF_UP))


def parse_time(value: Any, label: str) -> int:
    """Parse EPUB/SMIL clock values into integer milliseconds."""
    if value is None:
        raise LabError("missing %s" % label)
    raw = str(value).strip()
    if not raw:
        raise LabError("empty %s" % label)
    lowered = raw.lower()
    if ":" not in raw:
        for suffix, multiplier in (
            ("ms", Decimal("1")),
            ("min", Decimal("60000")),
            ("h", Decimal("3600000")),
            ("s", Decimal("1000")),
        ):
            if lowered.endswith(suffix):
                number = finite_decimal(raw[: -len(suffix)], label) * multiplier
                return int(number.quantize(Decimal("1"), rounding=ROUND_HALF_UP))
        return decimal_ms(raw, label)
    pieces = raw.split(":")
    if len(pieces) != 3:
        raise LabError("unsupported %s time %r" % (label, raw))
    hours = finite_decimal(pieces[0], label)
    minutes = finite_decimal(pieces[1], label)
    seconds = finite_decimal(pieces[2], label)
    if hours < 0 or minutes < 0 or minutes >= 60 or seconds < 0 or seconds >= 60:
        raise LabError("invalid %s time %r" % (label, raw))
    return int(((hours * 3600 + minutes * 60 + seconds) * 1000).quantize(Decimal("1"), rounding=ROUND_HALF_UP))


def format_seconds(milliseconds: int) -> str:
    return format(Decimal(milliseconds) / Decimal("1000"), "f")


def normalized_text(value: str) -> str:
    collapsed = " ".join(value.split()).casefold()
    return unicodedata.normalize("NFKC", collapsed)


def safe_member(path: str, label: str) -> str:
    split = urlsplit(path)
    if split.scheme or split.netloc or split.path.startswith("/"):
        raise LabError("%s is not an archive-local href: %r" % (label, path))
    decoded = unquote(split.path)
    normalized = posixpath.normpath(decoded)
    if normalized in ("", "."):
        raise LabError("%s resolves to an empty archive path: %r" % (label, path))
    if normalized == ".." or normalized.startswith("../"):
        raise LabError("%s escapes the archive: %r" % (label, path))
    return normalized


def resolve_href(base_entry: str, href: str, label: str) -> tuple[str, str]:
    split = urlsplit(href)
    if split.scheme or split.netloc or split.path.startswith("/"):
        raise LabError("%s is not archive-local: %r" % (label, href))
    path = unquote(split.path)
    base = posixpath.dirname(base_entry)
    joined = safe_member(posixpath.join(base, path), label)
    return joined, unquote(split.fragment)


def run_json_command(argv: list[str], label: str) -> dict[str, Any]:
    try:
        completed = subprocess.run(
            argv,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as exc:
        raise LabError("cannot run %s (%s): %s" % (label, argv[0], exc)) from exc
    if completed.returncode != 0:
        detail = completed.stderr.strip().splitlines()[-1:] or ["no diagnostic"]
        raise LabError("%s failed with exit %d: %s" % (label, completed.returncode, detail[0]))
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise LabError("%s returned invalid JSON: %s" % (label, exc)) from exc
    if not isinstance(value, dict):
        raise LabError("%s returned a non-object JSON value" % label)
    return value


def probe_audio(path: Path, ffprobe: str) -> dict[str, Any]:
    payload = run_json_command(
        [ffprobe, "-v", "error", "-print_format", "json", "-show_chapters", "-show_format", str(path)],
        "ffprobe",
    )
    raw_chapters = payload.get("chapters")
    if not isinstance(raw_chapters, list) or not raw_chapters:
        raise LabError("audio has no embedded chapters; complete-chapter slicing is unsafe")
    chapters: list[dict[str, Any]] = []
    previous_end = -1
    for index, chapter in enumerate(raw_chapters):
        if not isinstance(chapter, dict):
            raise LabError("ffprobe chapter %d is not an object" % index)
        start_value = chapter.get("start_time", chapter.get("start"))
        end_value = chapter.get("end_time", chapter.get("end"))
        start = decimal_ms(start_value, "chapter %d start" % index)
        end = decimal_ms(end_value, "chapter %d end" % index)
        if start < 0 or end <= start:
            raise LabError("chapter %d is not positive: %d..%d ms" % (index, start, end))
        if start < previous_end:
            raise LabError("audio chapters overlap or are out of order at %d" % index)
        tags = chapter.get("tags") if isinstance(chapter.get("tags"), dict) else {}
        title = str(tags.get("title") or "chapter-%d" % (index + 1))
        chapters.append({"index": index, "start_ms": start, "end_ms": end, "title": title})
        previous_end = end
    raw_format = payload.get("format") if isinstance(payload.get("format"), dict) else {}
    duration_value = raw_format.get("duration")
    duration_ms = decimal_ms(duration_value, "audio duration") if duration_value is not None else previous_end
    if duration_ms <= 0 or previous_end > duration_ms + 1000:
        raise LabError("chapter bounds exceed the finite audio duration")
    return {"duration_ms": duration_ms, "chapters": chapters}


def select_chapters(probe: dict[str, Any], fraction: Decimal) -> dict[str, Any]:
    if not fraction.is_finite() or fraction <= 0 or fraction > 1:
        raise LabError("target fraction must be finite, greater than 0, and no greater than 1")
    duration = int(probe["duration_ms"])
    target = Decimal(duration) * fraction
    chapters = probe["chapters"]
    candidates = []
    for count in range(1, len(chapters) + 1):
        end = int(chapters[count - 1]["end_ms"])
        distance = abs(Decimal(end) - target)
        candidates.append((distance, count, end))
    distance, count, end = min(candidates, key=lambda item: (item[0], item[1]))
    chosen = chapters[:count]
    start = int(chosen[0]["start_ms"])
    return {
        "target_fraction": str(fraction),
        "target_end_ms": int(target.quantize(Decimal("1"), rounding=ROUND_HALF_UP)),
        "selected_indices": [int(chapter["index"]) for chapter in chosen],
        "start_ms": start,
        "end_ms": end,
        "duration_ms": duration,
        "achieved_fraction": float(Decimal(end - start) / Decimal(duration)),
        "distance_ms": int(distance.quantize(Decimal("1"), rounding=ROUND_HALF_UP)),
        "chapters": chosen,
    }


def archive_infos(archive: zipfile.ZipFile) -> tuple[list[zipfile.ZipInfo], set[str]]:
    infos = archive.infolist()
    names = [info.filename for info in infos]
    if len(names) != len(set(names)):
        duplicates = sorted({name for name in names if names.count(name) > 1})
        raise LabError("EPUB has ambiguous duplicate ZIP entries: %s" % ", ".join(duplicates))
    return infos, set(names)


def parse_package(archive: zipfile.ZipFile, strict_mimetype: bool = True) -> dict[str, Any]:
    infos, names = archive_infos(archive)
    if not infos or infos[0].filename != "mimetype":
        if strict_mimetype:
            raise LabError("EPUB mimetype is not the first ZIP entry")
    elif strict_mimetype and infos[0].compress_type != zipfile.ZIP_STORED:
        raise LabError("EPUB mimetype must be stored, not deflated")
    if "mimetype" not in names or archive.read("mimetype") != MIMETYPE.encode("ascii"):
        raise LabError("EPUB mimetype entry is not application/epub+zip")
    if "META-INF/container.xml" not in names:
        raise LabError("EPUB lacks META-INF/container.xml")
    container_root = read_xml(archive.read("META-INF/container.xml"), "META-INF/container.xml")
    rootfiles = [node for node in container_root.iter() if local_name(node.tag) == "rootfile"]
    candidates = [attr(node, "full-path") for node in rootfiles if attr(node, "full-path")]
    if len(candidates) != 1:
        raise LabError("EPUB container must have exactly one rootfile; found %d" % len(candidates))
    opf_path = safe_member(candidates[0], "container rootfile")
    if opf_path not in names:
        raise LabError("container rootfile is missing: %s" % opf_path)
    opf_root = read_xml(archive.read(opf_path), opf_path)
    package_nodes = [node for node in opf_root.iter() if local_name(node.tag) == "package"]
    if not package_nodes:
        raise LabError("OPF has no package element")
    manifest_nodes = [node for node in opf_root.iter() if local_name(node.tag) == "item"]
    manifest: dict[str, dict[str, Any]] = {}
    for node in manifest_nodes:
        item_id = attr(node, "id")
        href = attr(node, "href")
        if not item_id or not href:
            raise LabError("OPF manifest item lacks id or href")
        if item_id in manifest:
            raise LabError("OPF manifest id is ambiguous: %s" % item_id)
        target, _ = resolve_href(opf_path, href, "manifest href")
        manifest[item_id] = {
            "id": item_id,
            "href": href,
            "path": target,
            "media_type": attr(node, "media-type") or "",
            "properties": tokens(attr(node, "properties")),
            "media_overlay": attr(node, "media-overlay"),
        }
    spine_nodes = [node for node in opf_root.iter() if local_name(node.tag) == "spine"]
    if len(spine_nodes) != 1:
        raise LabError("OPF must have exactly one spine; found %d" % len(spine_nodes))
    itemrefs = []
    for node in list(spine_nodes[0]):
        if local_name(node.tag) != "itemref":
            continue
        idref = attr(node, "idref")
        if not idref or idref not in manifest:
            raise LabError("spine itemref has no unique manifest target: %r" % idref)
        itemrefs.append({"element": node, "idref": idref, "linear": attr(node, "linear") != "no"})
    if len({item["idref"] for item in itemrefs}) != len(itemrefs):
        raise LabError("OPF spine contains duplicate idrefs")
    return {
        "infos": infos,
        "names": names,
        "container_root": container_root,
        "opf_path": opf_path,
        "opf_root": opf_root,
        "spine": spine_nodes[0],
        "itemrefs": itemrefs,
        "manifest": manifest,
    }


def nav_landmarks(archive: zipfile.ZipFile, package: dict[str, Any]) -> tuple[list[str], dict[str, list[str]]]:
    nav_items = [item for item in package["manifest"].values() if "nav" in item["properties"]]
    if len(nav_items) > 1:
        raise LabError("OPF navigation item is ambiguous")
    if not nav_items:
        return [], {}
    nav_item = nav_items[0]
    if nav_item["path"] not in package["names"]:
        raise LabError("OPF navigation document is missing")
    root = read_xml(archive.read(nav_item["path"]), nav_item["path"])
    bodymatter: list[str] = []
    labels: dict[str, list[str]] = {}
    for element in root.iter():
        if local_name(element.tag) != "a":
            continue
        href = attr(element, "href")
        if not href:
            continue
        target, _ = resolve_href(nav_item["path"], href, "navigation href")
        labels.setdefault(target, []).append(text_content(element))
        kind = attr(element, "type", NS_EPUB)
        if "bodymatter" in tokens(kind):
            bodymatter.append(target)
    return bodymatter, labels


def explicit_spine_ref(package: dict[str, Any], value: str, role: str) -> dict[str, Any]:
    candidates = []
    for item in package["itemrefs"]:
        manifest = package["manifest"][item["idref"]]
        href_path, _ = resolve_href(package["opf_path"], manifest["href"], "manifest href")
        if value == item["idref"] or value == manifest["href"] or value == href_path or value == manifest["path"]:
            candidates.append(item)
    if len(candidates) != 1:
        raise LabError("explicit %s is ambiguous or missing: %s" % (role, value))
    if not candidates[0]["linear"]:
        raise LabError("%s cannot be a non-linear spine item: %s" % (role, value))
    return candidates[0]


def detect_narrative_start(archive: zipfile.ZipFile, package: dict[str, Any], explicit: Optional[str]) -> tuple[dict[str, Any], str]:
    if explicit:
        return explicit_spine_ref(package, explicit, "narrative start"), "explicit"
    landmarks, _ = nav_landmarks(archive, package)
    matches = []
    for target in landmarks:
        for item in package["itemrefs"]:
            manifest = package["manifest"][item["idref"]]
            if manifest["path"] == target and item["linear"]:
                matches.append(item)
    unique = {item["idref"]: item for item in matches}
    if len(unique) > 1:
        raise LabError("navigation landmarks identify multiple narrative starts")
    if len(unique) == 1:
        return next(iter(unique.values())), "nav-landmark"
    linear = [item for item in package["itemrefs"] if item["linear"]]
    if not linear:
        raise LabError("OPF has no linear spine item for narrative start detection")
    return linear[0], "first-linear"


def labels_for_spine(archive: zipfile.ZipFile, package: dict[str, Any]) -> dict[str, list[str]]:
    _, nav_labels = nav_landmarks(archive, package)
    labels: dict[str, list[str]] = {}
    for position, item in enumerate(package["itemrefs"]):
        manifest = package["manifest"][item["idref"]]
        values = [item["idref"], manifest["href"], manifest["path"], posixpath.basename(manifest["path"])]
        values.extend(nav_labels.get(manifest["path"], []))
        labels[item["idref"]] = [value for value in values if value]
    return labels
def chapter_descriptor(value: str) -> Optional[tuple[str, int]]:
    match = re.search(r"(chapter|ch\.?|part|section)\s*[-_. ]*([0-9]+)", value, re.IGNORECASE)
    return (match.group(1).lower(), int(match.group(2))) if match else None




def choose_spine_end(
    archive: zipfile.ZipFile,
    package: dict[str, Any],
    selection: dict[str, Any],
    explicit_start: Optional[str],
    explicit_end: Optional[str],
) -> dict[str, Any]:
    start, start_method = detect_narrative_start(archive, package, explicit_start)
    refs = package["itemrefs"]
    start_position = next(index for index, item in enumerate(refs) if item["idref"] == start["idref"])
    linear_after = [
        (index, item) for index, item in enumerate(refs) if index >= start_position and item["linear"]
    ]
    explicit_end_position = None
    if explicit_end:
        end = explicit_spine_ref(package, explicit_end, "narrative end")
        explicit_end_position = next(index for index, item in enumerate(refs) if item["idref"] == end["idref"])
        if explicit_end_position < start_position:
            raise LabError("explicit narrative end precedes narrative start")
    labels = labels_for_spine(archive, package)
    descriptors = [chapter_descriptor(str(chapter["title"])) for chapter in selection["chapters"]]
    if any(descriptor is not None for descriptor in descriptors) and not all(descriptor is not None for descriptor in descriptors):
        raise LabError("selected audio chapters mix numbered and unnumbered titles; refusing partial matching")
    matched: list[tuple[int, str, int]] = []
    if any(descriptor is not None for descriptor in descriptors):
        for chapter, descriptor in zip(selection["chapters"], descriptors):
            if descriptor is None:
                continue
            candidates = []
            for index, item in linear_after:
                if any(chapter_descriptor(label) == descriptor for label in labels[item["idref"]]):
                    candidates.append((index, item))
            if len(candidates) > 1:
                raise LabError("audio chapter %d matches multiple EPUB spine items" % chapter["index"])
            if len(candidates) == 1:
                matched.append((candidates[0][0], candidates[0][1]["idref"], descriptor[1]))
        if matched and len(matched) != sum(descriptor is not None for descriptor in descriptors):
            raise LabError("audio chapter to EPUB spine matching is partial; refusing an unsafe slice")
    if matched:
        positions = [entry[0] for entry in matched]
        if positions != sorted(positions) or len(set(positions)) != len(positions):
            raise LabError("audio chapter to EPUB spine matching is not strictly ordered")
        end_position = positions[-1]
        if explicit_end_position is not None and explicit_end_position != end_position:
            raise LabError("explicit narrative end disagrees with exact chapter-number matching")
        method = "chapter-number"
    elif explicit_end_position is not None:
        end_position = explicit_end_position
        method = "explicit-range"
    else:
        raise LabError(
            "no exact audio chapter labels matched the EPUB spine; pass --narrative-start and --narrative-end"
        )
    kept = [item["idref"] for item in refs[: end_position + 1]]
    for item in refs[end_position + 1 :]:
        package["spine"].remove(item["element"])
    return {
        "method": method,
        "narrative_start": {"idref": start["idref"], "method": start_method, "spine_index": start_position},
        "narrative_end": {"idref": refs[end_position]["idref"], "spine_index": end_position},
        "spine_end_index": end_position,
        "spine_ids": kept,
        "chapter_matches": [
            {"audio_index": chapter["index"], "number": number, "spine_idref": spine_idref}
            for chapter, (_, spine_idref, number) in zip(selection["chapters"], matched)
        ],
    }


def serialize_xml(root: ET.Element) -> bytes:
    ET.register_namespace("", NS_OPF)
    ET.register_namespace("dc", NS_DC)
    ET.register_namespace("dcterms", NS_DCTERMS)
    ET.register_namespace("epub", NS_EPUB)
    return ET.tostring(root, encoding="utf-8", xml_declaration=True)


def deterministic_zip_info(name: str, compress_type: int) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = compress_type
    info.create_system = 3
    info.external_attr = 0o600 << 16
    info.flag_bits = 0x800
    return info


def rewrite_epub(source: Path, target: Path, package: dict[str, Any]) -> None:
    rewritten = serialize_xml(package["opf_root"])
    with zipfile.ZipFile(source, "r") as source_zip, zipfile.ZipFile(target, "w", allowZip64=True, compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        infos, _ = archive_infos(source_zip)
        ordered = sorted(infos, key=lambda info: (info.filename != "mimetype", info.filename))
        for source_info in ordered:
            compress_type = zipfile.ZIP_STORED if source_info.filename == "mimetype" else zipfile.ZIP_DEFLATED
            output_info = deterministic_zip_info(source_info.filename, compress_type)
            if source_info.filename == package["opf_path"]:
                output.writestr(output_info, rewritten)
                continue
            with source_zip.open(source_info, "r") as input_stream, output.open(output_info, "w", force_zip64=True) as output_stream:
                shutil.copyfileobj(input_stream, output_stream, length=1024 * 1024)


def ffmetadata_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace("=", "\\=").replace(";", "\\;").replace("#", "\\#").replace("\n", "\\n")


def write_ffmetadata(path: Path, chapters: Iterable[dict[str, Any]], selected_start: int) -> None:
    lines = [";FFMETADATA1", ""]
    for chapter in chapters:
        start = int(chapter["start_ms"]) - selected_start
        end = int(chapter["end_ms"]) - selected_start
        if start < 0 or end <= start:
            raise LabError("selected chapter cannot be rebased into positive ffmetadata bounds")
        lines.extend(
            [
                "[CHAPTER]",
                "TIMEBASE=1/1000",
                "START=%d" % start,
                "END=%d" % end,
                "title=%s" % ffmetadata_escape(str(chapter["title"])),
                "",
            ]
        )
    path.write_text("\n".join(lines), encoding="utf-8", newline="\n")


def output_name(value: str, default: str) -> str:
    name = value or default
    if Path(name).name != name or name in ("", ".", ".."):
        raise LabError("output names must be simple files inside the output directory")
    return name


def refuse_overwrite(paths: Iterable[Path], force: bool) -> None:
    for path in paths:
        if path.exists() and not force:
            raise LabError("refusing to overwrite existing output: %s (pass --force)" % path)
        if path.exists() and path.is_dir():
            raise LabError("output path is a directory: %s" % path)




def slice_fixture(args: argparse.Namespace) -> dict[str, Any]:
    epub = Path(args.epub)
    audio = Path(args.audio)
    if not epub.is_file() or not audio.is_file():
        raise LabError("slice inputs must be existing files")
    output_dir = Path(args.out_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    epub_output = output_dir / output_name(args.epub_name, "fixture.epub")
    audio_output = output_dir / output_name(args.audio_name, "fixture.m4b")
    manifest_output = Path(args.manifest) if args.manifest else output_dir / "fixture.json"
    outputs = [epub_output, audio_output, manifest_output]
    if len({path.resolve() for path in outputs}) != len(outputs):
        raise LabError("EPUB, audio, and manifest outputs must be distinct")
    refuse_overwrite(outputs, args.force)
    if any(path.resolve() in {epub.resolve(), audio.resolve()} for path in outputs):
        raise LabError("outputs must differ from source files; originals are immutable")
    try:
        fraction = Decimal(str(args.fraction))
    except InvalidOperation as exc:
        raise LabError("fraction is not a decimal") from exc
    probe = probe_audio(audio, args.ffprobe)
    selection = select_chapters(probe, fraction)
    with zipfile.ZipFile(epub, "r") as archive:
        package = parse_package(archive, strict_mimetype=True)
        spine = choose_spine_end(
            archive, package, selection, args.narrative_start, args.narrative_end
        )
    source_epub_hash = sha256_file(epub)
    source_audio_hash = sha256_file(audio)
    with tempfile.TemporaryDirectory(prefix="stump-read-aloud-lab-", dir=str(output_dir.parent)) as temporary_dir:
        staging = Path(temporary_dir)
        staged_epub = staging / epub_output.name
        staged_audio = staging / audio_output.name
        ffmeta = staging / "chapters.ffmeta"
        write_ffmetadata(ffmeta, selection["chapters"], selection["start_ms"])
        rewrite_epub(epub, staged_epub, package)
        try:
            ffmpeg_result = subprocess.run(
                [
                    args.ffmpeg,
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-ss",
                    format_seconds(selection["start_ms"]),
                    "-t",
                    format_seconds(selection["end_ms"] - selection["start_ms"]),
                    "-i",
                    str(audio),
                    "-f",
                    "ffmetadata",
                    "-i",
                    str(ffmeta),
                    "-map",
                    "0:a:0",
                    "-map",
                    "0:v:0?",
                    "-map_metadata",
                    "0",
                    "-map_chapters",
                    "1",
                    "-c",
                    "copy",
                    str(staged_audio),
                ],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
        except OSError as exc:
            raise LabError("cannot run ffmpeg (%s): %s" % (args.ffmpeg, exc)) from exc
        if ffmpeg_result.returncode != 0:
            detail = ffmpeg_result.stderr.strip().splitlines()[-1:] or ["no diagnostic"]
            raise LabError("ffmpeg failed with exit %d: %s" % (ffmpeg_result.returncode, detail[0]))
        if not staged_audio.is_file() or staged_audio.stat().st_size == 0:
            raise LabError("ffmpeg exited successfully but produced no audio output")
        manifest = {
            "schema": "stump-read-aloud-fixture/v1",
            "tool": {"name": "read-aloud-lab", "version": LAB_VERSION},
            "mode": args.mode,
            "private": True,
            "selection": {
                "target_fraction": selection["target_fraction"],
                "target_end_ms": selection["target_end_ms"],
                "selected_audio_chapters": selection["selected_indices"],
                "start_ms": selection["start_ms"],
                "end_ms": selection["end_ms"],
                "duration_ms": selection["duration_ms"],
                "achieved_fraction": selection["achieved_fraction"],
                "distance_ms": selection["distance_ms"],
                "epub_spine_ids": spine["spine_ids"],
                "spine_match": spine,
            },
            "source": {
                "epub": {"path": str(epub), "sha256": source_epub_hash},
                "audiobook": {"path": str(audio), "sha256": source_audio_hash},
            },
            "fixture": {
                "epub": {"path": str(epub_output), "sha256": sha256_file(staged_epub)},
                "audiobook": {"path": str(audio_output), "sha256": sha256_file(staged_audio)},
            },
            "provenance": {
                "ffprobe": args.ffprobe,
                "ffmpeg": args.ffmpeg,
                "stream_copy": True,
                "ffmetadata_sha256": sha256_file(ffmeta),
                "chapter_boundaries_ms": [
                    {"index": c["index"], "start": c["start_ms"], "end": c["end_ms"]} for c in selection["chapters"]
                ],
            },
        }
        manifest_bytes = json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8") + b"\n"
        staged_manifest = staging / manifest_output.name
        staged_manifest.write_bytes(manifest_bytes)
        os.replace(staged_epub, epub_output)
        os.replace(staged_audio, audio_output)
        manifest_output.parent.mkdir(parents=True, exist_ok=True)
        os.replace(staged_manifest, manifest_output)
    return manifest


def package_duration(package: dict[str, Any]) -> Optional[int]:
    values = []
    for node in package["opf_root"].iter():
        if local_name(node.tag) != "meta":
            continue
        if attr(node, "property") == "media:duration" and not attr(node, "refines"):
            raw = attr(node, "content") or (node.text or "").strip()
            if raw:
                values.append(parse_time(raw, "unrefined media:duration"))
    if not values:
        return None
    if len(set(values)) != 1:
        raise LabError("OPF has conflicting unrefined media:duration metadata")
    return values[0]


def parse_audio_target(archive: zipfile.ZipFile, package: dict[str, Any], smil_path: str, audio_src: str) -> tuple[str, str]:
    target, fragment = resolve_href(smil_path, audio_src, "SMIL audio target")
    if target not in package["names"]:
        raise LabError("SMIL audio target is missing: %s" % target)
    return target, fragment


def text_target_exists(
    archive: zipfile.ZipFile,
    package: dict[str, Any],
    smil_path: str,
    text_src: str,
    cache: dict[str, ET.Element],
) -> tuple[str, str, str]:
    target, fragment = resolve_href(smil_path, text_src, "SMIL text target")
    if not fragment:
        raise LabError("SMIL text target must include an element fragment")
    if target not in package["names"]:
        raise LabError("SMIL text target is missing: %s" % target)
    if target not in cache:
        cache[target] = read_xml(archive.read(target), target)
    matches = [
        node
        for node in cache[target].iter()
        if attr(node, "id") == fragment or attr(node, "id", NS_XML) == fragment
    ]
    if len(matches) != 1:
        raise LabError("SMIL text fragment is ambiguous or missing: %s#%s" % (target, fragment))
    normalized = normalized_text(text_content(matches[0]))
    if not normalized:
        raise LabError("SMIL text fragment has no comparable text: %s#%s" % (target, fragment))
    return target, fragment, sha256_bytes(normalized.encode("utf-8"))


def parse_cues(archive: zipfile.ZipFile, package: dict[str, Any]) -> list[dict[str, Any]]:
    duration = package_duration(package)
    if duration is None:
        raise LabError("OPF lacks one unrefined finite media:duration value")
    smil_items = [item for item in package["manifest"].values() if item["media_type"] == "application/smil+xml"]
    if not smil_items:
        raise LabError("OPF contains no SMIL overlays")
    overlay_ids = {item["id"] for item in smil_items}
    referenced_overlays: set[str] = set()
    for manifest_item in package["manifest"].values():
        overlay = manifest_item["media_overlay"]
        if not overlay:
            continue
        if overlay not in overlay_ids:
            raise LabError("spine media-overlay target is missing: %s" % overlay)
        referenced_overlays.add(overlay)
    if referenced_overlays != overlay_ids:
        raise LabError("OPF contains an orphan or unreferenced SMIL overlay")
    cache: dict[str, ET.Element] = {}
    cues: list[dict[str, Any]] = []
    seen_text_targets: set[str] = set()
    for smil in sorted(smil_items, key=lambda item: item["id"]):
        smil_path = smil["path"]
        if smil_path not in package["names"]:
            raise LabError("SMIL entry is missing: %s" % smil_path)
        root = read_xml(archive.read(smil_path), smil_path)
        previous: dict[str, int] = {}
        for ordinal, par in enumerate(node for node in root.iter() if local_name(node.tag) == "par"):
            text_nodes = [node for node in list(par) if local_name(node.tag) == "text"]
            audio_nodes = [node for node in list(par) if local_name(node.tag) == "audio"]
            if len(text_nodes) != 1 or len(audio_nodes) != 1:
                raise LabError("SMIL par %d in %s must have one text and one audio child" % (ordinal, smil_path))
            text_src = attr(text_nodes[0], "src")
            audio_src = attr(audio_nodes[0], "src")
            if not text_src or not audio_src:
                raise LabError("SMIL par %d in %s lacks text/audio src" % (ordinal, smil_path))
            text_path, fragment, text_sha256 = text_target_exists(
                archive, package, smil_path, text_src, cache
            )
            text_key = "%s#%s" % (text_path, fragment)
            if text_key in seen_text_targets:
                raise LabError("SMIL text target is used by more than one cue: %s" % text_key)
            seen_text_targets.add(text_key)
            audio_path, _ = parse_audio_target(archive, package, smil_path, audio_src)
            begin = parse_time(attr(audio_nodes[0], "clipBegin"), "clipBegin")
            end = parse_time(attr(audio_nodes[0], "clipEnd"), "clipEnd")
            if begin < 0 or end <= begin:
                raise LabError("SMIL cue %d in %s is not positive" % (ordinal, smil_path))
            if end > duration:
                raise LabError("SMIL cue %d in %s exceeds media:duration" % (ordinal, smil_path))
            previous_end = previous.get(audio_path)
            if previous_end is not None and begin < previous_end:
                raise LabError("SMIL cues overlap or are non-monotonic in %s" % smil_path)
            previous[audio_path] = end
            cues.append(
                {
                    "key": text_key,
                    "text_path": text_path,
                    "text_sha256": text_sha256,
                    "smil": smil_path,
                    "audio": audio_path,
                    "begin_ms": begin,
                    "end_ms": end,
                }
            )
    if not cues:
        raise LabError("SMIL overlays contain no cues")
    by_audio: dict[str, list[dict[str, Any]]] = {}
    for cue in cues:
        by_audio.setdefault(cue["audio"], []).append(cue)
    for audio_path, audio_cues in by_audio.items():
        ordered = sorted(audio_cues, key=lambda cue: (cue["begin_ms"], cue["end_ms"]))
        for previous_cue, cue in zip(ordered, ordered[1:]):
            if cue["begin_ms"] < previous_cue["end_ms"]:
                raise LabError("SMIL cues overlap across overlays for %s" % audio_path)
    return cues


def inspect_epub(path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    result: dict[str, Any] = {
        "schema": "stump-read-aloud-inspection/v1",
        "path": str(path),
        "valid": False,
        "errors": [],
        "zip": {},
        "opf": {},
        "overlays": {},
        "targets": {},
    }
    if not path.is_file():
        result["errors"] = ["EPUB does not exist: %s" % path]
        return result, []
    result["sha256"] = sha256_file(path)
    try:
        with zipfile.ZipFile(path, "r") as archive:
            infos, names = archive_infos(archive)
            first = infos[0] if infos else None
            result["zip"] = {
                "entries": len(infos),
                "mimetype_first": bool(first and first.filename == "mimetype"),
                "mimetype_stored": bool(first and first.filename == "mimetype" and first.compress_type == zipfile.ZIP_STORED),
                "mimetype_value": archive.read("mimetype").decode("ascii", errors="replace") if "mimetype" in names else None,
            }
            package = parse_package(archive, strict_mimetype=True)
            result["opf"] = {"path": package["opf_path"], "spine_items": len(package["itemrefs"]), "manifest_items": len(package["manifest"])}
            cues = parse_cues(archive, package)
            result["overlays"] = {
                "smil_files": len([item for item in package["manifest"].values() if item["media_type"] == "application/smil+xml"]),
                "cues": len(cues),
                "media_duration_ms": package_duration(package),
            }
            result["targets"] = {
                "text": len({cue["key"] for cue in cues}),
                "audio": len({cue["audio"] for cue in cues}),
                "resolved": len(cues),
            }
            result["valid"] = True
            return result, cues
    except (zipfile.BadZipFile, OSError, LabError) as exc:
        result["errors"] = [str(exc)]
        return result, []


def percentile(values: list[int], percentile_value: int) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    rank = (len(ordered) - 1) * percentile_value
    lower = int(math.floor(rank / 100))
    upper = int(math.ceil(rank / 100))
    if lower == upper:
        return float(ordered[lower])
    weight = (rank / 100) - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * weight


def compact_number(value: Optional[float]) -> Optional[int | float]:
    if value is None:
        return None
    rounded = round(value, 3)
    return int(rounded) if rounded.is_integer() else rounded


def compare_epubs(left: Path, right: Path) -> dict[str, Any]:
    left_result, left_cues = inspect_epub(left)
    right_result, right_cues = inspect_epub(right)
    if not left_result["valid"]:
        raise LabError("left EPUB is invalid: %s" % "; ".join(left_result["errors"]))
    if not right_result["valid"]:
        raise LabError("right EPUB is invalid: %s" % "; ".join(right_result["errors"]))
    left_map: dict[str, list[dict[str, Any]]] = {}
    right_map: dict[str, list[dict[str, Any]]] = {}
    for cue in left_cues:
        semantic_key = "%s:%s" % (cue["text_path"], cue["text_sha256"])
        left_map.setdefault(semantic_key, []).append(cue)
    for cue in right_cues:
        semantic_key = "%s:%s" % (cue["text_path"], cue["text_sha256"])
        right_map.setdefault(semantic_key, []).append(cue)
    common = sorted(set(left_map) & set(right_map))
    begin_deltas: list[int] = []
    end_deltas: list[int] = []
    matched = 0
    for key in common:
        left_group = left_map[key]
        right_group = right_map[key]
        if len(left_group) != len(right_group):
            raise LabError("common text target has different cue cardinality: %s" % key)
        for left_cue, right_cue in zip(left_group, right_group):
            begin_deltas.append(abs(int(left_cue["begin_ms"]) - int(right_cue["begin_ms"])))
            end_deltas.append(abs(int(left_cue["end_ms"]) - int(right_cue["end_ms"])))
            matched += 1
    left_only = sum(len(left_map[key]) for key in set(left_map) - set(right_map))
    right_only = sum(len(right_map[key]) for key in set(right_map) - set(left_map))
    return {
        "schema": "stump-read-aloud-comparison/v1",
        "left": {"path": str(left), "sha256": left_result["sha256"]},
        "right": {"path": str(right), "sha256": right_result["sha256"]},
        "common_text_targets": matched,
        "matched_cues": matched,
        "unmatched_text_targets": {
            "left": left_only,
            "right": right_only,
        },
        "timing_delta_ms": {
            "begin": {
                "median": compact_number(percentile(begin_deltas, 50)),
                "p95": compact_number(percentile(begin_deltas, 95)),
                "max": max(begin_deltas) if begin_deltas else None,
            },
            "end": {
                "median": compact_number(percentile(end_deltas, 50)),
                "p95": compact_number(percentile(end_deltas, 95)),
                "max": max(end_deltas) if end_deltas else None,
            },
        },
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="read-aloud-lab.py",
        description="Deterministic private EPUB/MP4 read-aloud fixture slicing, inspection, and comparison.",
    )
    commands = parser.add_subparsers(dest="command", required=True)

    slice_parser = commands.add_parser("slice", help="slice whole audio chapters and the EPUB spine into a fixture")
    slice_parser.add_argument("--epub", required=True, help="source EPUB (never modified)")
    slice_parser.add_argument("--audio", required=True, help="source M4B/M4A (never modified)")
    slice_parser.add_argument("--out-dir", required=True, help="mode-specific output directory")
    slice_parser.add_argument("--fraction", default="0.10", help="target fraction of audio duration (default: 0.10)")
    slice_parser.add_argument("--narrative-start", help="explicit spine idref or href; otherwise detect a unique bodymatter landmark")
    slice_parser.add_argument("--narrative-end", help="explicit inclusive spine idref or href when exact chapter labels cannot be matched")
    slice_parser.add_argument("--mode", choices=("shared", "whisper", "ctc"), help="fixture mode recorded in the manifest")
    slice_parser.add_argument("--epub-name", default="fixture.epub", help="output EPUB basename")
    slice_parser.add_argument("--audio-name", default="fixture.m4b", help="output audio basename")
    slice_parser.add_argument("--manifest", help="manifest path (default: <out-dir>/fixture.json)")
    slice_parser.add_argument("--ffprobe", default="ffprobe", help="ffprobe executable")
    slice_parser.add_argument("--ffmpeg", default="ffmpeg", help="ffmpeg executable")
    slice_parser.add_argument("--force", action="store_true", help="allow replacement of the three named outputs")
    slice_parser.set_defaults(handler=slice_fixture)

    inspect_parser = commands.add_parser("inspect", help="validate EPUB packaging, OPF overlays, SMIL targets, and cues")
    inspect_parser.add_argument("epub", help="EPUB to inspect")
    inspect_parser.set_defaults(handler=lambda args: inspect_epub(Path(args.epub))[0])

    compare_parser = commands.add_parser("compare", help="compare cue timings for common text targets in two EPUBs")
    compare_parser.add_argument("left", help="first Storyteller/SMIL EPUB")
    compare_parser.add_argument("right", help="second Storyteller/SMIL EPUB")
    compare_parser.set_defaults(handler=lambda args: compare_epubs(Path(args.left), Path(args.right)))
    return parser


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        value = args.handler(args)
        emit_json(value)
        if args.command == "inspect" and not value.get("valid", False):
            return 1
        return 0
    except LabError as exc:
        emit_json({"schema": "stump-read-aloud-lab-error/v1", "error": str(exc)})
        return 2
    except (OSError, ValueError, zipfile.BadZipFile) as exc:
        emit_json({"schema": "stump-read-aloud-lab-error/v1", "error": str(exc)})
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
