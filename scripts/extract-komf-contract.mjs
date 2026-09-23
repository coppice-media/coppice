#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const UPSTREAMS_PATH = resolve(ROOT, "scripts/upstreams.json");
const CONTRACT_PATH = resolve(ROOT, "scripts/contracts/komf-media-server.json");
const SCHEMA = "coppice/komf-media-server-contract-v1";
const KOMF_REPO = "Snd-R/komf";
const KOMGA_CLIENT_REPO = "Snd-R/komga-client";
const REQUEST_TIMEOUT_MS = 20_000;
const MAX_GITHUB_RESPONSE_BYTES = 8 * 1024 * 1024;
const MAX_TREE_ENTRIES = 50_000;
const MAX_SOURCE_FILES = 128;
const MAX_SOURCE_FILE_BYTES = 1024 * 1024;
const CLASS_DECLARATIONS = new Set(["class", "object"]);

class ContractError extends Error {
  constructor(message) {
    super(message);
    this.name = "ContractError";
  }
}

function usage() {
  console.log(`Usage: node scripts/extract-komf-contract.mjs [mode]\n\nModes (exactly one):\n  --check   Extract the pinned source and compare the checked-in manifest.\n  --write   Extract the pinned source and write the checked-in manifest.\n  --latest  Inspect the current upstream head without changing the manifest.\n  --help    Show this help (also the default when no mode is given).\n\nThe canonical source is always the exact Komf commit in scripts/upstreams.json.\n`);
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new ContractError(`Cannot read JSON ${path}: ${error.message}`);
  }
}

function stableValue(value) {
  if (Array.isArray(value)) return value.map(stableValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, stableValue(value[key])]),
    );
  }
  return value;
}

function stableJson(value) {
  return `${JSON.stringify(stableValue(value), null, 2)}\n`;
}

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function githubViaGh(path) {
  const output = execFileSync("gh", ["api", path], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: REQUEST_TIMEOUT_MS,
    maxBuffer: MAX_GITHUB_RESPONSE_BYTES,
  });
  return JSON.parse(output);
}

async function boundedResponseText(response, path) {
  const contentLength = Number(response.headers.get("content-length") ?? 0);
  if (contentLength > MAX_GITHUB_RESPONSE_BYTES) {
    await response.body?.cancel();
    throw new ContractError(`${path}: GitHub response exceeds ${MAX_GITHUB_RESPONSE_BYTES} bytes`);
  }
  if (!response.body) return "";
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_GITHUB_RESPONSE_BYTES) {
      await reader.cancel();
      throw new ContractError(`${path}: GitHub response exceeds ${MAX_GITHUB_RESPONSE_BYTES} bytes`);
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks, size).toString("utf8");
}

async function githubJson(path) {
  try {
    return githubViaGh(path);
  } catch {
    const response = await fetch(`https://api.github.com/${path}`, {
      headers: {
        Accept: "application/vnd.github+json",
        "User-Agent": "coppice-komf-contract-extractor",
      },
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    const body = await boundedResponseText(response, path);
    if (!response.ok) {
      throw new ContractError(
        `${path}: GitHub API ${response.status} ${response.statusText}: ${body.slice(0, 300)}`,
      );
    }
    try {
      return JSON.parse(body);
    } catch (error) {
      throw new ContractError(`${path}: invalid GitHub JSON: ${error.message}`);
    }
  }
}

async function sourceText(repo, ref, entry) {
  if (!Number.isInteger(entry.size) || entry.size < 0 || entry.size > MAX_SOURCE_FILE_BYTES) {
    throw new ContractError(`${repo}@${ref}:${entry.path}: source size is outside the supported bound`);
  }
  const blob = await githubJson(`repos/${repo}/git/blobs/${entry.sha}`);
  if (blob.sha !== entry.sha || blob.encoding !== "base64" || typeof blob.content !== "string") {
    throw new ContractError(`${repo}@${ref}:${entry.path}: GitHub returned an unexpected blob`);
  }
  const encoded = blob.content.replace(/\s+/g, "");
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded)) {
    throw new ContractError(`${repo}@${ref}:${entry.path}: GitHub returned malformed base64 blob content`);
  }
  const bytes = Buffer.from(encoded, "base64");
  if (bytes.byteLength !== entry.size || blob.size !== entry.size) {
    throw new ContractError(`${repo}@${ref}:${entry.path}: GitHub blob size does not match the tree entry`);
  }
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch (error) {
    throw new ContractError(`${repo}@${ref}:${entry.path}: source is not valid UTF-8: ${error.message}`);
  }
}

function isIdentifierStart(character) {
  return /[A-Za-z_]/.test(character) || character === "`";
}

function isIdentifierPart(character) {
  return /[A-Za-z0-9_]/.test(character);
}

function decodeKotlinString(body) {
  return body.replace(/\\(u[0-9a-fA-F]{4}|.)/g, (match, escaped) => {
    if (escaped.startsWith("u")) return String.fromCharCode(Number.parseInt(escaped.slice(1), 16));
    return (
      {
        n: "\n",
        r: "\r",
        t: "\t",
        b: "\b",
        f: "\f",
        "\\": "\\",
        '"': '"',
        "'": "'",
      }[escaped] ?? escaped
    );
  });
}

/**
 * Kotlin lexer used by the extractor. It deliberately emits punctuation and
 * skips comments/strings as units so delimiter matching cannot be confused by
 * braces in comments, quoted paths, or string templates.
 */
function lexKotlin(source, path) {
  const tokens = [];
  let index = 0;
  let line = 1;

  const add = (kind, value, start, end, raw) => {
    tokens.push({ kind, value, start, end, raw, line });
  };

  while (index < source.length) {
    const character = source[index];
    if (/\s/.test(character)) {
      if (character === "\n") line += 1;
      index += 1;
      continue;
    }

    if (source.startsWith("//", index)) {
      const newline = source.indexOf("\n", index + 2);
      if (newline === -1) break;
      index = newline + 1;
      line += 1;
      continue;
    }

    if (source.startsWith("/*", index)) {
      const start = index;
      let depth = 1;
      index += 2;
      while (index < source.length && depth > 0) {
        if (source.startsWith("/*", index)) {
          depth += 1;
          index += 2;
        } else if (source.startsWith("*/", index)) {
          depth -= 1;
          index += 2;
        } else {
          if (source[index] === "\n") line += 1;
          index += 1;
        }
      }
      if (depth !== 0) throw new ContractError(`${path}:${line}: unterminated block comment`);
      void start;
      continue;
    }

    if (source.startsWith("\"\"\"", index)) {
      const start = index;
      const bodyStart = index + 3;
      const close = source.indexOf("\"\"\"", bodyStart);
      if (close === -1) throw new ContractError(`${path}:${line}: unterminated triple-quoted string`);
      const body = source.slice(bodyStart, close);
      line += (body.match(/\n/g) ?? []).length;
      index = close + 3;
      add("string", body, start, index, source.slice(start, index));
      continue;
    }

    if (character === '"' || character === "'") {
      const quote = character;
      const start = index;
      index += 1;
      let body = "";
      let closed = false;
      while (index < source.length) {
        const current = source[index];
        if (current === "\\") {
          if (index + 1 >= source.length) break;
          body += source.slice(index, index + 2);
          index += 2;
          continue;
        }
        if (current === quote) {
          index += 1;
          closed = true;
          break;
        }
        if (current === "\n") line += 1;
        body += current;
        index += 1;
      }
      if (!closed) throw new ContractError(`${path}:${line}: unterminated quoted string`);
      add("string", quote === '"' ? decodeKotlinString(body) : body, start, index, source.slice(start, index));
      continue;
    }

    if (isIdentifierStart(character)) {
      const start = index;
      if (character === "`") {
        index += 1;
        const close = source.indexOf("`", index);
        if (close === -1) throw new ContractError(`${path}:${line}: unterminated backtick identifier`);
        index = close + 1;
        add("identifier", source.slice(start + 1, close), start, index, source.slice(start, index));
        continue;
      }
      index += 1;
      while (index < source.length && isIdentifierPart(source[index])) index += 1;
      add("identifier", source.slice(start, index), start, index, source.slice(start, index));
      continue;
    }

    if (/[0-9]/.test(character)) {
      const start = index;
      index += 1;
      while (index < source.length && /[A-Za-z0-9_.]/.test(source[index])) index += 1;
      add("number", source.slice(start, index), start, index, source.slice(start, index));
      continue;
    }
    const two = source.slice(index, index + 2);
    if (["?.", "::", "->", "=>", "!!", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/="].includes(two)) {
      add("symbol", two, index, index + 2, two);
      index += 2;
      continue;
    }

    add("symbol", character, index, index + 1, character);
    index += 1;
  }
  return tokens;
}

function matchingDelimiters(tokens, path) {
  const openToClose = new Map();
  const closeToOpen = new Map();
  const stack = [];
  const openers = new Map([["(", ")"], ["[", "]"], ["{", "}"]]);
  const closers = new Set([")", "]", "}"]);

  for (let index = 0; index < tokens.length; index += 1) {
    const value = tokens[index].value;
    if (openers.has(value)) {
      stack.push({ value, index });
      continue;
    }
    if (!closers.has(value)) continue;
    const opening = stack.pop();
    if (!opening || openers.get(opening.value) !== value) {
      throw new ContractError(`${path}:${tokens[index].line}: unmatched delimiter ${value}`);
    }
    openToClose.set(opening.index, index);
    closeToOpen.set(index, opening.index);
  }
  if (stack.length > 0) {
    const opening = stack[stack.length - 1];
    throw new ContractError(`${path}:${tokens[opening.index].line}: unmatched delimiter ${opening.value}`);
  }
  return { openToClose, closeToOpen };
}

function tokenText(source, tokens, start, end) {
  if (start >= end) return "";
  return source.slice(tokens[start].start, tokens[end - 1].end).trim();
}

function normalizedType(value) {
  return value.replace(/\s+/g, "").replace(/\?$/, "?");
}

function splitTopLevel(tokens, start, end, { typeArguments = false } = {}) {
  const parts = [];
  let partStart = start;
  let parens = 0;
  let brackets = 0;
  let braces = 0;
  let angles = 0;
  for (let index = start; index < end; index += 1) {
    switch (tokens[index].value) {
      case "(": parens += 1; break;
      case ")": parens -= 1; break;
      case "[": brackets += 1; break;
      case "]": brackets -= 1; break;
      case "{": braces += 1; break;
      case "}": braces -= 1; break;
      case "<":
        if (typeArguments) angles += 1;
        break;
      case ">":
        if (typeArguments && angles > 0) angles -= 1;
        break;
      case ",":
        if (parens === 0 && brackets === 0 && braces === 0 && angles === 0) {
          parts.push([partStart, index]);
          partStart = index + 1;
        }
        break;
      default: break;
    }
  }
  if (partStart < end) parts.push([partStart, end]);
  return parts;
}

function parseParameters(source, tokens, openIndex, closeIndex) {
  return splitTopLevel(tokens, openIndex + 1, closeIndex, { typeArguments: true })
    .map(([start, end]) => {
      while (start < end && tokens[start].value === ",") start += 1;
      while (end > start && tokens[end - 1].value === ",") end -= 1;
      if (start >= end) return null;
      let colon = -1;
      let equals = -1;
      let depth = 0;
      for (let index = start; index < end; index += 1) {
        const value = tokens[index].value;
        if (["(", "[", "{"].includes(value)) depth += 1;
        else if ([")", "]", "}"].includes(value)) depth -= 1;
        else if (depth === 0 && value === ":" && colon === -1) colon = index;
        else if (depth === 0 && value === "=" && equals === -1) equals = index;
      }
      if (colon === -1) throw new ContractError(`${source.slice(tokens[start].start, tokens[end - 1].end)}: parameter has no type`);
      const nameToken = tokens[colon - 1];
      const name = nameToken?.value;
      if (!name || nameToken.kind !== "identifier") {
        throw new ContractError(`${source.slice(tokens[start].start, tokens[end - 1].end)}: parameter has no name`);
      }
      const typeEnd = equals === -1 ? end : equals;
      const type = tokenText(source, tokens, colon + 1, typeEnd);
      if (!type) throw new ContractError(`${name}: parameter has no type`);
      return { name, type, normalizedType: normalizedType(type) };
    })
    .filter(Boolean);
}

function findFunctionName(tokens, funIndex, limit) {
  for (let index = funIndex + 1; index < limit; index += 1) {
    if (tokens[index].value === "(") {
      const candidate = tokens[index - 1];
      if (!candidate || candidate.kind !== "identifier") continue;
      return { name: candidate.value, openIndex: index };
    }
    if (["{", "=", ";"].includes(tokens[index].value)) break;
  }
  throw new ContractError(`${tokens[funIndex].line}: unable to parse Kotlin function name`);
}

function parseFunction(source, tokens, pairs, funIndex, limit) {
  const { name, openIndex } = findFunctionName(tokens, funIndex, limit);
  const closeIndex = pairs.openToClose.get(openIndex);
  if (closeIndex == null || closeIndex >= limit) {
    throw new ContractError(`${tokens[funIndex].line}: function ${name} has an invalid parameter list`);
  }
  const parameters = parseParameters(source, tokens, openIndex, closeIndex);
  let bodyOpen = null;
  let expressionIndex = null;
  let index = closeIndex + 1;
  while (index < limit) {
    if (tokens[index].value === "{") {
      bodyOpen = index;
      break;
    }
    if (tokens[index].value === "=") {
      expressionIndex = index;
      break;
    }
    if (tokens[index].value === "fun") break;
    index += 1;
  }
  let bodyClose = null;
  let body = null;
  let endIndex = index;
  if (bodyOpen != null) {
    bodyClose = pairs.openToClose.get(bodyOpen);
    if (bodyClose == null || bodyClose >= limit) {
      throw new ContractError(`${tokens[funIndex].line}: function ${name} has an invalid body`);
    }
    body = {
      openIndex: bodyOpen,
      closeIndex: bodyClose,
      startIndex: bodyOpen + 1,
      endIndex: bodyClose,
      start: tokens[bodyOpen].end,
      end: tokens[bodyClose].start,
      text: source.slice(tokens[bodyOpen].end, tokens[bodyClose].start),
    };
    endIndex = bodyClose + 1;
  } else if (expressionIndex != null) {
    let expressionEnd = expressionIndex + 1;
    while (expressionEnd < limit && tokens[expressionEnd].value !== "fun") expressionEnd += 1;
    body = {
      openIndex: null,
      closeIndex: null,
      startIndex: expressionIndex + 1,
      endIndex: expressionEnd,
      start: tokens[expressionIndex].end,
      end: expressionEnd < limit ? tokens[expressionEnd].start : source.length,
      text: source.slice(tokens[expressionIndex].end, expressionEnd < limit ? tokens[expressionEnd].start : source.length),
    };
    endIndex = expressionEnd;
  }
  let returnType = null;
  const returnLimit = bodyOpen ?? expressionIndex ?? limit;
  let returnEnd = returnLimit;
  if (bodyOpen == null && expressionIndex == null) {
    for (let cursor = closeIndex + 1; cursor < limit; cursor += 1) {
      if (tokens[cursor].value === "fun") {
        returnEnd = cursor;
        break;
      }
    }
  }
  for (let cursor = closeIndex + 1; cursor < returnEnd; cursor += 1) {
    if (tokens[cursor].value === ":") {
      returnType = tokenText(source, tokens, cursor + 1, returnEnd);
      break;
    }
  }
  return {
    name,
    parameters,
    returnType,
    body,
    startIndex: funIndex,
    endIndex,
    signature: tokenText(source, tokens, funIndex, bodyOpen ?? expressionIndex ?? endIndex),
    line: tokens[funIndex].line,
  };
}

function directDepth(tokens, openIndex, closeIndex, index) {
  let depth = 0;
  for (let cursor = openIndex + 1; cursor < index && cursor < closeIndex; cursor += 1) {
    if (tokens[cursor].value === "{") depth += 1;
    else if (tokens[cursor].value === "}") depth -= 1;
  }
  return depth;
}

function parseInterface(source, tokens, pairs, path) {
  const interfaceIndexes = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value === "interface" && tokens[index + 1]?.value === "MediaServerClient") {
      interfaceIndexes.push(index);
    }
  }
  if (interfaceIndexes.length !== 1) {
    throw new ContractError(`${path}: expected one MediaServerClient interface, found ${interfaceIndexes.length}`);
  }
  const index = interfaceIndexes[0];
  let bodyOpen = index + 2;
  while (bodyOpen < tokens.length && tokens[bodyOpen].value !== "{") bodyOpen += 1;
  if (tokens[bodyOpen]?.value !== "{") {
    throw new ContractError(`${path}:${tokens[index].line}: MediaServerClient has no body`);
  }
  const bodyClose = pairs.openToClose.get(bodyOpen);
  if (bodyClose == null) throw new ContractError(`${path}:${tokens[index].line}: MediaServerClient has no balanced body`);
  const declarations = [];
  const operationIds = new Set();
  for (let cursor = bodyOpen + 1; cursor < bodyClose; cursor += 1) {
    if (tokens[cursor].value !== "fun" || directDepth(tokens, bodyOpen, bodyClose, cursor) !== 0) continue;
    const operation = parseFunction(source, tokens, pairs, cursor, bodyClose);
    if (operation.body) {
      throw new ContractError(`${path}:${operation.line}: interface operation ${operation.name} unexpectedly has a body`);
    }
    const id = operationId(operation);
    if (operationIds.has(id)) throw new ContractError(`${path}:${operation.line}: duplicate MediaServerClient operation ${id}`);
    operationIds.add(id);
    declarations.push(operation);
    cursor = Math.max(cursor, operation.endIndex - 1);
  }
  if (declarations.length === 0) throw new ContractError(`${path}: MediaServerClient has no operations`);
  return { bodyOpen, bodyClose, operations: declarations };
}

function parseClasses(source, tokens, pairs, path) {
  const classes = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (!CLASS_DECLARATIONS.has(tokens[index].value)) continue;
    const nameToken = tokens[index + 1];
    if (!nameToken || nameToken.kind !== "identifier") {
      if (tokens[index].value === "class" && tokens[index - 1]?.value !== "::") {
        throw new ContractError(`${path}:${tokens[index].line}: class declaration has no name`);
      }
      if (tokens[index].value === "object" && tokens[index + 1]?.value === ":") {
        let headerEnd = index + 2;
        while (headerEnd < tokens.length && tokens[headerEnd].value !== "{") {
          if (["class", "object", "interface", "fun"].includes(tokens[headerEnd].value)) break;
          headerEnd += 1;
        }
        if (tokens.slice(index, headerEnd).some((token) => token.value === "MediaServerClient")) {
          throw new ContractError(`${path}:${tokens[index].line}: anonymous MediaServerClient adapters are unsupported`);
        }
      }
      continue;
    }
    let bodyOpen = index + 2;
    let parens = 0;
    let brackets = 0;
    let braces = 0;
    let angles = 0;
    while (bodyOpen < tokens.length) {
      const value = tokens[bodyOpen].value;
      if (value === "{" && parens === 0 && brackets === 0 && braces === 0 && angles === 0) break;
      if (
        parens === 0 &&
        brackets === 0 &&
        braces === 0 &&
        angles === 0 &&
        ["class", "object", "interface", "fun", "val", "var", "typealias"].includes(value) &&
        bodyOpen > index + 2
      ) break;
      if (value === "(") parens += 1;
      else if (value === ")") parens -= 1;
      else if (value === "[") brackets += 1;
      else if (value === "]") brackets -= 1;
      else if (value === "{") braces += 1;
      else if (value === "}") braces -= 1;
      else if (value === "<") angles += 1;
      else if (value === ">" && angles > 0) angles -= 1;
      bodyOpen += 1;
    }
    const header = tokens.slice(index, bodyOpen);
    // Constructor parameter and generic-bound types are not implemented
    // interfaces; only the top-level supertype list counts.
    let headerParens = 0;
    let headerAngles = 0;
    let supertypeStart = -1;
    for (let cursor = 0; cursor < header.length; cursor += 1) {
      const value = header[cursor].value;
      if (value === "(") headerParens += 1;
      else if (value === ")") headerParens -= 1;
      else if (value === "<") headerAngles += 1;
      else if (value === ">" && headerAngles > 0) headerAngles -= 1;
      else if (value === ":" && headerParens === 0 && headerAngles === 0) {
        supertypeStart = cursor + 1;
        break;
      }
    }
    const implementsInterface =
      supertypeStart !== -1 && header.slice(supertypeStart).some((token) => token.value === "MediaServerClient");
    if (tokens[bodyOpen]?.value !== "{") {
      if (implementsInterface) {
        throw new ContractError(`${path}:${tokens[index].line}: MediaServerClient adapter ${nameToken.value} has no body`);
      }
      continue;
    }
    const bodyClose = pairs.openToClose.get(bodyOpen);
    if (bodyClose == null) throw new ContractError(`${path}:${tokens[index].line}: class ${nameToken.value} has no balanced body`);
    const properties = {};
    for (let cursor = index; cursor < bodyOpen - 2; cursor += 1) {
      if (!["val", "var"].includes(tokens[cursor].value)) continue;
      const property = tokens[cursor + 1];
      if (!property || property.kind !== "identifier") continue;
      let type = null;
      if (tokens[cursor + 2]?.value === ":") {
        let typeEnd = cursor + 3;
        while (typeEnd < bodyOpen && ![",", ")", "=", "val", "var"].includes(tokens[typeEnd].value)) typeEnd += 1;
        type = tokenText(source, tokens, cursor + 3, typeEnd);
      }
      if (type) properties[property.value] = normalizedType(type);
    }
    const methods = [];
    let cursor = bodyOpen + 1;
    while (cursor < bodyClose) {
      if (tokens[cursor].value === "fun" && directDepth(tokens, bodyOpen, bodyClose, cursor) === 0) {
        const method = parseFunction(source, tokens, pairs, cursor, bodyClose);
        const prefixStart = methods.length === 0 ? bodyOpen + 1 : methods[methods.length - 1].endIndex;
        const prefix = tokens.slice(prefixStart, cursor).map((token) => token.value);
        method.overridden = prefix.includes("override");
        method.private = prefix.includes("private");
        methods.push(method);
        cursor = Math.max(cursor, method.endIndex);
      } else {
        cursor += 1;
      }
    }
    classes.push({
      name: nameToken.value,
      path,
      implementsInterface,
      properties,
      methods,
      startIndex: index,
      bodyOpen,
      bodyClose,
      headerText: tokenText(source, tokens, index, bodyOpen),
    });
  }
  return classes;
}


function operationId(operation) {
  return `${operation.name}(${operation.parameters.map((parameter) => parameter.normalizedType).join(",")})`;
}

function callArguments(tokens, pairs, openIndex) {
  const closeIndex = pairs.openToClose.get(openIndex);
  if (closeIndex == null) return { closeIndex: null, ranges: [] };
  return { closeIndex, ranges: splitTopLevel(tokens, openIndex + 1, closeIndex) };
}

function callAt(tokens, pairs, index) {
  let openIndex = index + 1;
  if (tokens[openIndex]?.value === "<") {
    let angle = 1;
    openIndex += 1;
    while (openIndex < tokens.length && angle > 0) {
      if (tokens[openIndex].value === "<") angle += 1;
      else if (tokens[openIndex].value === ">") angle -= 1;
      openIndex += 1;
    }
  }
  if (tokens[openIndex]?.value !== "(") return null;
  const { closeIndex, ranges } = callArguments(tokens, pairs, openIndex);
  if (closeIndex == null) return null;
  return { openIndex, closeIndex, ranges };
}

function valueFromRange(tokens, range) {
  const [start, end] = range;
  if (end - start === 1 && tokens[start].kind === "string") return tokens[start].value;
  return null;
}

function normalizePath(value) {
  if (!value) return value;
  const withSlash = value.startsWith("/") ? value : `/${value}`;
  return withSlash.replace(/\/+/g, "/").replace(/\/\/$/, "") || "/";
}

function expressionStartBefore(tokens, pairs, endIndex) {
  if (endIndex <= 0) return 0;
  const last = endIndex - 1;
  const value = tokens[last].value;
  if (value === ")") {
    const openIndex = pairs.closeToOpen.get(last);
    if (openIndex == null) return last;
    let calleeEnd = openIndex;
    if (tokens[openIndex - 1]?.value === ">") {
      let angleDepth = 0;
      for (let cursor = openIndex - 1; cursor >= 0; cursor -= 1) {
        if (tokens[cursor].value === ">") angleDepth += 1;
        else if (tokens[cursor].value === "<" && --angleDepth === 0) {
          calleeEnd = cursor;
          break;
        }
      }
    }
    return expressionStartBefore(tokens, pairs, calleeEnd);
  }
  if (value === "]") {
    const openIndex = pairs.closeToOpen.get(last);
    return openIndex == null ? last : expressionStartBefore(tokens, pairs, openIndex);
  }
  if (tokens[last].kind === "identifier") {
    let start = last;
    while (start >= 2 && [".", "?."].includes(tokens[start - 1]?.value)) {
      start = expressionStartBefore(tokens, pairs, start - 1);
    }
    return start;
  }
  return last;
}

function receiverFor(tokens, pairs, index) {
  const previous = tokens[index - 1]?.value;
  if (previous !== "." && previous !== "?.") return null;
  const receiverEnd = index - 2;
  const start = expressionStartBefore(tokens, pairs, receiverEnd + 1);
  const receiverTokens = tokens.slice(start, receiverEnd + 1);
  if (receiverTokens.some((token) => ["(", ")", "[", "]"].includes(token.value))) return "<expression>";
  const names = receiverTokens.filter((token) => token.kind === "identifier").map((token) => token.value);
  return names.join(".") || "<expression>";
}

function sourceSlice(source, tokens, startIndex, endIndex) {
  if (startIndex == null || endIndex == null || startIndex >= endIndex) return "";
  return source.slice(tokens[startIndex].start, tokens[endIndex - 1].end);
}

function extractCalls(source, tokens, pairs, startIndex = 0, endIndex = tokens.length) {
  const calls = [];
  for (let index = startIndex; index < endIndex; index += 1) {
    if (tokens[index].kind !== "identifier") continue;
    const call = callAt(tokens, pairs, index);
    if (!call) continue;
    if (tokens[index - 1]?.value === "fun") continue;
    const receiver = receiverFor(tokens, pairs, index);
    const args = call.ranges.map((range) => sourceSlice(source, tokens, range[0], range[1]).trim());
    const expressionStart = receiver ? expressionStartBefore(tokens, pairs, index - 1) : index;
    calls.push({
      name: tokens[index].value,
      receiver,
      args,
      expression: sourceSlice(source, tokens, expressionStart, call.closeIndex + 1).trim(),
      index,
      call,
    });
  }
  return calls;
}

function directStringEndpoints(source, tokens, pairs, startIndex, endIndex, context = {}) {
  const endpoints = [];
  const requestVerbs = new Map([
    ["get", "GET"], ["post", "POST"], ["put", "PUT"], ["patch", "PATCH"],
    ["delete", "DELETE"], ["head", "HEAD"], ["options", "OPTIONS"], ["sse", "SSE"],
  ]);
  const routeNames = new Set(["route", "path", "url", "sse"]);
  for (let index = startIndex; index < endIndex; index += 1) {
    if (tokens[index].kind !== "identifier") continue;
    const name = tokens[index].value;
    const call = callAt(tokens, pairs, index);
    if (!call) continue;
    const first = call.ranges.length > 0 ? valueFromRange(tokens, call.ranges[0]) : null;
    if (first != null && (requestVerbs.has(name) || routeNames.has(name))) {
      const verb = name === "route" ? "ROUTE" : name === "path" ? "PATH" : name === "url" ? "URL" : requestVerbs.get(name);
      endpoints.push({
        verb,
        path: normalizePath(first),
        literal: first,
        sourcePath: context.sourcePath,
        line: tokens[index].line,
        function: context.functionName ?? null,
      });
    }
    if (name === "appendPathSegments" && call.ranges.length > 0) {
      const segments = call.ranges.map((range) => valueFromRange(tokens, range)).filter((value) => value != null);
      if (segments.length === call.ranges.length) {
        endpoints.push({
          verb: "PATH",
          path: normalizePath(segments.join("/")),
          literal: segments.join("/"),
          sourcePath: context.sourcePath,
          line: tokens[index].line,
          function: context.functionName ?? null,
        });
      }
    }
    if (requestVerbs.has(name) && tokens[index + 1]?.value === "{") {
      const bodyClose = pairs.openToClose.get(index + 1);
      if (bodyClose == null || bodyClose >= endIndex) continue;
      const nested = directStringEndpoints(source, tokens, pairs, index + 2, bodyClose, context);
      const nestedPath = nested.find((endpoint) => ["PATH", "URL"].includes(endpoint.verb));
      if (nestedPath) {
        endpoints.push({
          verb: requestVerbs.get(name),
          path: nestedPath.path,
          literal: nestedPath.literal,
          sourcePath: context.sourcePath,
          line: tokens[index].line,
          function: context.functionName ?? null,
        });
      }
    }
    if (name === "sseSession" && tokens[index + 1]?.value === "{") {
      const bodyClose = pairs.openToClose.get(index + 1);
      if (bodyClose == null || bodyClose >= endIndex) continue;
      const nested = directStringEndpoints(source, tokens, pairs, index + 2, bodyClose, context);
      const nestedUrl = nested.find((endpoint) => endpoint.verb === "URL");
      if (nestedUrl) {
        endpoints.push({
          verb: "SSE",
          path: nestedUrl.path,
          literal: nestedUrl.literal,
          sourcePath: context.sourcePath,
          line: tokens[index].line,
          function: context.functionName ?? null,
        });
      }
    }
  }
  return endpoints;
}

function endpointsForFunction(source, tokens, pairs, method, context) {
  if (!method.body) return [];
  return directStringEndpoints(source, tokens, pairs, method.body.startIndex, method.body.endIndex, {
    ...context,
    functionName: method.name,
  });
}

function endpointsForSource(parsed) {
  const endpoints = [];
  const context = { sourcePath: parsed.entry.path };
  for (const classDeclaration of parsed.classes) {
    for (const method of classDeclaration.methods) {
      endpoints.push(...endpointsForFunction(parsed.text, parsed.tokens, parsed.pairs, method, context));
    }
  }
  const classRanges = parsed.classes.map((item) => [item.bodyOpen, item.bodyClose]);
  for (let index = 0; index < parsed.tokens.length; index += 1) {
    if (parsed.tokens[index].value !== "fun") continue;
    if (classRanges.some(([start, end]) => index > start && index < end)) continue;
    const method = parseFunction(parsed.text, parsed.tokens, parsed.pairs, index, parsed.tokens.length);
    if (method.body) {
      endpoints.push(...endpointsForFunction(parsed.text, parsed.tokens, parsed.pairs, method, context));
    }
    index = Math.max(index, method.endIndex - 1);
  }
  return endpoints;
}

function noOpMethod(method, calls) {
  const bodyText = method.body?.text?.replace(/\/\/.*$/gm, "").trim() ?? "";
  if (!method.body) return false;
  if (!bodyText) return true;
  if (/^(?:return\s+)?(?:emptyList\(\)|emptySet\(\)|null|Unit|TODO\([^)]*\))$/.test(bodyText.replace(/\s+/g, " "))) return true;
  return calls.length === 0 && /^\{?\s*\}?$/.test(bodyText);
}

function eventNames(source, tokens, pairs, path) {
  const handled = new Set();
  const noOp = new Set();
  for (let index = 0; index < tokens.length - 3; index += 1) {
    if (tokens[index].value === "KomgaEvent" && tokens[index + 1]?.value === "." && tokens[index + 2]?.kind === "identifier") {
      handled.add(tokens[index + 2].value);
    }
    if (tokens[index].value === "on" && tokens[index + 1]?.value === "(" && tokens[index + 2]?.kind === "string") {
      handled.add(tokens[index + 2].value);
    }
    if (tokens[index].value === "noopEvents" && tokens[index + 1]?.value === "=" && tokens[index + 2]?.value === "listOf") {
      const open = index + 3;
      if (tokens[open]?.value !== "(") continue;
      const close = pairs.openToClose.get(open);
      if (close == null) throw new ContractError(`${path}:${tokens[index].line}: noopEvents has no balanced list`);
      for (const [start, end] of splitTopLevel(tokens, open + 1, close)) {
        const value = valueFromRange(tokens, [start, end]);
        if (value != null) noOp.add(value);
      }
    }
  }
  return { handled: [...handled].sort(), noOp: [...noOp].sort() };
}

function findClassMethod(classes, typeName, methodName, argumentCount) {
  const declarations = classes.filter((item) => item.name === typeName);
  if (declarations.length !== 1) return null;
  const classDeclaration = declarations[0];
  const candidates = classDeclaration.methods.filter(
    (method) => method.name === methodName && method.parameters.length === argumentCount,
  );
  return candidates.length === 1 ? { classDeclaration, method: candidates[0] } : null;
}
function adapterView(adapter, parsed, interfaceOperations, allClasses) {
  const overrideMethods = adapter.methods.filter((method) => method.overridden);
  const byId = new Map(interfaceOperations.map((operation) => [operationId(operation), operation]));
  const mapped = new Map();
  const operationRecords = [];
  for (const method of overrideMethods) {
    const id = operationId(method);
    const interfaceOperation = byId.get(id);
    if (!interfaceOperation) {
      throw new ContractError(`${parsed.entry.path}:${method.line}: adapter ${adapter.name} operation ${id} is not in MediaServerClient`);
    }
    const key = operationId(interfaceOperation);
    if (mapped.has(key)) throw new ContractError(`${parsed.entry.path}:${method.line}: duplicate adapter operation ${key}`);
    mapped.set(key, method);
    const calls = method.body
      ? extractCalls(parsed.text, parsed.tokens, parsed.pairs, method.body.startIndex, method.body.endIndex)
        .filter((call) => call.receiver)
        .map((call) => {
          const receiverBase = call.receiver.split(".")[0];
          const receiverType = adapter.properties[receiverBase] ?? null;
          const externalKomga = receiverType?.startsWith("Komga") || receiverBase.startsWith("komga");
          const resolved = receiverType ? findClassMethod(allClasses, receiverType, call.name, call.args.length) : null;
          let endpoints = [];
          if (!externalKomga && resolved) {
            endpoints = endpointsForFunction(
              resolved.classDeclaration.source,
              resolved.classDeclaration._tokens,
              resolved.classDeclaration._pairs,
              resolved.method,
              { sourcePath: resolved.classDeclaration.path },
            );
          }
          return {
            argumentCount: call.args.length,
            endpoints,
            expression: call.expression,
            name: call.name,
            receiver: call.receiver,
            receiverType,
            resolution: externalKomga ? "unresolved-external-komga-client" : resolved ? "resolved-source" : "unresolved",
            sourcePath: resolved?.classDeclaration.path ?? null,
          };
        })
      : [];
    const noOp = noOpMethod(method, calls);
    operationRecords.push({
      clientCalls: calls,
      implementation: {
        line: method.line,
        name: method.name,
        parameters: method.parameters,
        signature: method.signature.replace(/\s+/g, " ").trim(),
      },
      noOp,
      operation: key,
      sourcePath: parsed.entry.path,
    });
  }
  for (const operation of interfaceOperations) {
    if (!mapped.has(operationId(operation))) {
      throw new ContractError(`${parsed.entry.path}: adapter ${adapter.name} does not implement ${operationId(operation)}`);
    }
  }
  if (overrideMethods.length !== interfaceOperations.length) {
    throw new ContractError(`${parsed.entry.path}: adapter ${adapter.name} has ${overrideMethods.length} overrides for ${interfaceOperations.length} interface operations`);
  }
  operationRecords.sort((left, right) => compareText(left.operation, right.operation));
  return operationRecords;
}


function sourceRecord(repo, ref, entry, text) {
  return {
    commit: ref,
    gitBlobSha: entry.sha,
    path: entry.path,
    repository: repo,
    sha256: sha256(text),
    url: `https://github.com/${repo}/blob/${ref}/${entry.path}`,
  };
}

function parseSource(repo, ref, entry, text) {
  const tokens = lexKotlin(text, entry.path);
  const pairs = matchingDelimiters(tokens, entry.path);
  const classes = parseClasses(text, tokens, pairs, entry.path);
  for (const classDeclaration of classes) {
    classDeclaration._tokens = tokens;
    classDeclaration._pairs = pairs;
    classDeclaration.source = text;
  }
  return { repo, ref, entry, text, tokens, pairs, classes };
}

function interfaceOperationView(operation, source) {
  return {
    id: operationId(operation),
    name: operation.name,
    parameters: operation.parameters,
    returnType: operation.returnType,
    signature: operation.signature.replace(/\s+/g, " ").trim(),
    line: operation.line,
    sourcePath: source.entry.path,
  };
}


function parseVersion(catalogSource, path, key) {
  const match = catalogSource.match(new RegExp(`^${key.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\\\$&")}\\s*=\\s*[\"']([^\"']+)[\"']`, "m"));
  if (!match) throw new ContractError(`${path}: ${key} is missing`);
  return match[1];
}

async function fetchRepository(repo, ref, includeEntry) {
  if (typeof includeEntry !== "function") throw new ContractError(`${repo}@${ref}: source selector is required`);
  const tree = await githubJson(`repos/${repo}/git/trees/${ref}?recursive=1`);
  if (tree.truncated) throw new ContractError(`${repo}@${ref}: GitHub tree response was truncated`);
  if (!Array.isArray(tree.tree) || tree.tree.length > MAX_TREE_ENTRIES) {
    throw new ContractError(`${repo}@${ref}: GitHub tree is invalid or exceeds ${MAX_TREE_ENTRIES} entries`);
  }
  const entries = tree.tree
    .filter((entry) => entry.type === "blob" && entry.path.endsWith(".kt") && includeEntry(entry))
    .sort((left, right) => compareText(left.path, right.path));
  if (entries.length > MAX_SOURCE_FILES) {
    throw new ContractError(`${repo}@${ref}: selected ${entries.length} Kotlin files, exceeding limit ${MAX_SOURCE_FILES}`);
  }
  for (const entry of entries) {
    if (!/^[0-9a-f]{40}$/.test(entry.sha) || !Number.isInteger(entry.size) || entry.size < 0) {
      throw new ContractError(`${repo}@${ref}:${entry.path}: invalid Git tree blob metadata`);
    }
  }
  const cache = new Map();
  const get = async (entry) => {
    if (!cache.has(entry.path)) cache.set(entry.path, sourceText(repo, ref, entry));
    return cache.get(entry.path);
  };
  return { tree, entries, get };
}

async function extractContract({ komfEntry, ref, expectedVersion }) {
  const komfRepository = await fetchRepository(KOMF_REPO, ref, (entry) =>
    (entry.path.startsWith("komf-mediaserver/src/") && /\/[^/]*Main\/kotlin\//.test(entry.path)) ||
    (entry.path.startsWith("komf-app/src/main/kotlin/") && /\/(?:[^/]*Routes|ServerModule)\.kt$/.test(entry.path)),
  );
  const catalogEntry = komfRepository.tree.tree.find((entry) => entry.path === "gradle/libs.versions.toml");
  if (!catalogEntry) throw new ContractError(`${KOMF_REPO}@${ref}: gradle/libs.versions.toml not found`);
  const catalogSource = await komfRepository.get(catalogEntry);
  const version = parseVersion(catalogSource, catalogEntry.path, "app-version");
  const komgaClientVersion = parseVersion(catalogSource, catalogEntry.path, "komga-client");
  if (expectedVersion && version !== expectedVersion) {
    throw new ContractError(`${KOMF_REPO}@${ref}: expected Komf ${expectedVersion}, source reports ${version}`);
  }

  const selectedEntries = komfRepository.entries;
  const parsedSources = [];
  for (const entry of selectedEntries) {
    const text = await komfRepository.get(entry);
    parsedSources.push(parseSource(KOMF_REPO, ref, entry, text));
  }

  const interfaceSources = parsedSources.filter((item) => item.entry.path.endsWith("/MediaServerClient.kt"));
  if (interfaceSources.length !== 1) {
    throw new ContractError(`${KOMF_REPO}@${ref}: expected one MediaServerClient.kt, found ${interfaceSources.length}`);
  }
  const interfaceSource = interfaceSources[0];
  const interfaceDeclaration = parseInterface(interfaceSource.text, interfaceSource.tokens, interfaceSource.pairs, interfaceSource.entry.path);
  const interfaceOperations = interfaceDeclaration.operations.map((operation) => interfaceOperationView(operation, interfaceSource));
  const interfaceSourceRecord = sourceRecord(KOMF_REPO, ref, interfaceSource.entry, interfaceSource.text);

  const allClasses = parsedSources.flatMap((item) => item.classes);
  const adapterClasses = allClasses.filter((item) => item.implementsInterface);
  if (adapterClasses.length === 0) throw new ContractError(`${KOMF_REPO}@${ref}: no MediaServerClient adapter classes found`);
  const adapterLibraries = new Set();
  const adapters = adapterClasses.map((adapter) => {
    const suffix = "MediaServerClientAdapter";
    if (!adapter.name.endsWith(suffix)) {
      throw new ContractError(`${adapter.path}: adapter class ${adapter.name} must end with ${suffix}`);
    }
    const name = adapter.name.slice(0, -suffix.length).toLowerCase();
    if (!name) throw new ContractError(`${adapter.path}: adapter class has no library name`);
    if (adapterLibraries.has(name)) throw new ContractError(`${adapter.path}: duplicate MediaServerClient adapter for ${name}`);
    adapterLibraries.add(name);
    const parsed = parsedSources.find((item) => item.entry.path === adapter.path);
    if (!parsed) throw new ContractError(`${adapter.path}: adapter source was not selected`);
    const eventSources = parsedSources.filter(
      (item) => item.entry.path.toLowerCase().includes(`/${name}/`) && /EventHandler\.kt$/.test(item.entry.path),
    );
    if (eventSources.length > 1) throw new ContractError(`${adapter.path}: multiple ${name} EventHandler.kt sources found`);
    const eventSource = eventSources[0] ?? null;
    const events = eventSource ? eventNames(eventSource.text, eventSource.tokens, eventSource.pairs, eventSource.entry.path) : { handled: [], noOp: [] };
    return {
      adapter: adapter.name,
      class: adapter.name,
      library: name,
      noOpEvents: events.noOp,
      events: events.handled,
      eventSourcePath: eventSource?.entry.path ?? null,
      operations: adapterView(adapter, parsed, interfaceDeclaration.operations, allClasses),
      sourcePath: adapter.path,
    };
  }).sort((left, right) => compareText(left.library, right.library));

  const appParsed = parsedSources.filter((item) => item.entry.path.startsWith("komf-app/"));
  const appEndpoints = appParsed.flatMap(endpointsForSource);
  const appSources = appParsed.map((parsed) => sourceRecord(KOMF_REPO, ref, parsed.entry, parsed.text));
  const appRoutes = appEndpoints
    .map((endpoint) => ({
      function: endpoint.function,
      line: endpoint.line,
      path: endpoint.path,
      sourcePath: endpoint.sourcePath,
      verb: endpoint.verb,
    }))
    .sort((left, right) => compareText(`${left.sourcePath}:${left.line}:${left.verb}:${left.path}`, `${right.sourcePath}:${right.line}:${right.verb}:${right.path}`));
  const routePrefixes = [...new Set(appRoutes.filter((route) => route.verb === "ROUTE").map((route) => route.path))].sort();

  const externalRepoInfo = komfEntry.dependencies?.["komga-client"] ?? {};
  const externalRef = externalRepoInfo.baseline?.value;
  const expectedKomgaClientVersion = externalRepoInfo.version;
  if (!/^[0-9a-f]{40}$/.test(externalRef ?? "")) {
    throw new ContractError("Komf upstream entry is missing an exact dependencies.komga-client.baseline commit");
  }
  if (!expectedKomgaClientVersion) {
    throw new ContractError("Komf upstream entry is missing dependencies.komga-client.version");
  }
  if (komgaClientVersion !== expectedKomgaClientVersion) {
    throw new ContractError(
      `${KOMF_REPO}@${ref}: expected komga-client ${expectedKomgaClientVersion}, source reports ${komgaClientVersion}`,
    );
  }
  const externalRepository = await fetchRepository(KOMGA_CLIENT_REPO, externalRef, (entry) =>
    entry.path.startsWith("src/commonMain/kotlin/snd/komga/client/"),
  );
  const externalEntries = externalRepository.entries;
  if (externalEntries.length === 0) {
    throw new ContractError(`${KOMGA_CLIENT_REPO}@${externalRef}: no common Kotlin client sources found`);
  }
  const externalParsed = [];
  for (const entry of externalEntries) {
    const text = await externalRepository.get(entry);
    externalParsed.push(parseSource(KOMGA_CLIENT_REPO, externalRef, entry, text));
  }
  const externalRoutes = externalParsed.flatMap((parsed) => {
    if (!/(?:^|\/)(?:Http[^/]+|KomgaClientFactory)\.kt$/.test(parsed.entry.path)) return [];
    return endpointsForSource(parsed).map((route) => ({
      function: route.function,
      line: route.line,
      path: route.path,
      sourcePath: route.sourcePath,
      verb: route.verb,
    }));
  });
  const externalSourceRecords = externalParsed.map((parsed) => sourceRecord(KOMGA_CLIENT_REPO, externalRef, parsed.entry, parsed.text));
  const ssePaths = [...new Set(externalRoutes.filter((route) => route.verb === "SSE").map((route) => route.path))].sort();

  const sourceRecords = [interfaceSourceRecord, ...parsedSources.filter((parsed) => parsed !== interfaceSource).map((parsed) => sourceRecord(KOMF_REPO, ref, parsed.entry, parsed.text))]
    .sort((left, right) => compareText(`${left.repository}:${left.path}`, `${right.repository}:${right.path}`));
  const canonicalEntry = {
    baseline: komfEntry.baseline,
    commit: ref,
    repository: KOMF_REPO,
    version,
  };

  return stableValue({
    adapters,
    app: {
      endpoints: appRoutes,
      routePrefixes,
      sourcePaths: appSources.map((item) => item.path).sort(),
    },
    dependencies: {
      komgaClient: {
        commit: externalRef,
        repository: KOMGA_CLIENT_REPO,
        routes: externalRoutes.sort((left, right) => compareText(`${left.sourcePath}:${left.line}:${left.verb}:${left.path}`, `${right.sourcePath}:${right.line}:${right.verb}:${right.path}`)),
        sourceHashes: externalSourceRecords,
        ssePaths,
        version: komgaClientVersion,
      },
    },
    generatedBy: "scripts/extract-komf-contract.mjs",
    interface: {
      name: "MediaServerClient",
      operations: interfaceOperations.sort((left, right) => compareText(left.id, right.id)),
      source: interfaceSourceRecord,
    },
    schema: SCHEMA,
    source: canonicalEntry,
    sourceHashes: sourceRecords,
  });
}

function findUpstream(upstreams, repo) {
  if (!Array.isArray(upstreams.repositories)) {
    throw new ContractError("scripts/upstreams.json must contain a repositories array");
  }
  const matches = upstreams.repositories.filter((candidate) => candidate.repo === repo);
  if (matches.length !== 1) {
    throw new ContractError(`scripts/upstreams.json must contain exactly one ${repo} entry; found ${matches.length}`);
  }
  const entry = matches[0];
  if (entry.baseline?.kind !== "commit" || !/^[0-9a-f]{40}$/.test(entry.baseline.value ?? "")) {
    throw new ContractError(`${repo} baseline must be an exact 40-character commit`);
  }
  return entry;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0 || (args.length === 1 && args[0] === "--help")) {
    usage();
    return;
  }
  if (args.includes("--help")) throw new ContractError("--help cannot be combined with another mode");
  const allowed = new Set(["--check", "--write", "--latest"]);
  const unknown = args.filter((arg) => !allowed.has(arg));
  if (unknown.length > 0) throw new ContractError(`unknown option(s): ${unknown.join(", ")}`);
  if (args.length !== 1) throw new ContractError("choose exactly one of --check, --write, or --latest");

  const mode = args[0].slice(2);
  const upstreams = readJson(UPSTREAMS_PATH);
  const komfEntry = findUpstream(upstreams, KOMF_REPO);
  let ref = komfEntry.baseline.value;
  if (mode === "latest") {
    const repository = await githubJson(`repos/${KOMF_REPO}`);
    const branch = komfEntry.branch ?? repository.default_branch;
    if (!branch) throw new ContractError(`${KOMF_REPO}: no branch was available for latest inspection`);
    const head = await githubJson(`repos/${KOMF_REPO}/commits/${encodeURIComponent(branch)}`);
    if (!/^[0-9a-f]{40}$/.test(head.sha ?? "")) {
      throw new ContractError(`${KOMF_REPO}: GitHub did not return an exact latest commit SHA`);
    }
    ref = head.sha;
    const latestContract = await extractContract({ komfEntry, ref, expectedVersion: null });
    let canonical = null;
    try {
      canonical = JSON.parse(readFileSync(CONTRACT_PATH, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT") {
        throw new ContractError(`Cannot read JSON ${CONTRACT_PATH}: ${error.message}`);
      }
    }
    console.log(stableJson({
      canonicalCommit: komfEntry.baseline.value,
      changedFromCanonical: canonical ? stableJson(canonical) !== stableJson(latestContract) : true,
      inspectedCommit: ref,
      inspectedVersion: latestContract.source.version,
      mode: "latest",
      summary: {
        adapters: latestContract.adapters.map((adapter) => adapter.library),
        interfaceOperations: latestContract.interface.operations.length,
        komgaClientRoutes: latestContract.dependencies.komgaClient.routes.length,
      },
    }));
    return;
  }

  const contract = await extractContract({
    komfEntry,
    ref,
    expectedVersion: komfEntry.reviewedRelease ?? komfEntry.release?.version ?? null,
  });
  const generated = stableJson(contract);
  if (mode === "write") {
    mkdirSync(dirname(CONTRACT_PATH), { recursive: true });
    writeFileSync(CONTRACT_PATH, generated, "utf8");
    process.stdout.write(`Wrote ${CONTRACT_PATH}\n`);
    return;
  }
  let checked;
  try {
    checked = readFileSync(CONTRACT_PATH, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") {
      throw new ContractError(`checked-in contract is missing: ${CONTRACT_PATH}`);
    }
    throw new ContractError(`Cannot read checked-in contract ${CONTRACT_PATH}: ${error.message}`);
  }
  if (checked !== generated) {
    console.error(`Komf contract drift detected in ${CONTRACT_PATH}; run bun run update:komf-contract`);
    process.exitCode = 1;
    return;
  }
  console.log(`Komf contract is current at ${ref}`);
}

main().catch((error) => {
  console.error(`extract-komf-contract: ${error.message}`);
  process.exitCode = 1;
});
