#!/usr/bin/env bun

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
const KOMF_CLIENT_RELEASE = {
  ref: "3a0fb57028ef8235ea6bba6f399329da3c48b084",
  version: "2.0.0",
};
const KOMELIA_REPO = "Snd-R/Komelia";
const KOMF_CLIENT_REPO = KOMF_REPO;
const KOMF_CLIENT_SOURCE_PREFIX = "komf-client/src/commonMain/kotlin/snd/komf/client/";
const KOMF_API_MODEL_PREFIX = "komf-api-models/src/commonMain/kotlin/snd/komf/api/";
const KOMELIA_SOURCE_PREFIXES = [
  "komelia-komf-extension/content/src/wasmJsMain/kotlin/snd/komelia",
  "komelia-ui/src/commonMain/kotlin/snd/komelia/ui/settings/komf",
  "komelia-ui/src/commonMain/kotlin/snd/komelia/ui/dialogs/komf",
];
const REQUEST_TIMEOUT_MS = 20_000;
const MAX_GITHUB_RESPONSE_BYTES = 8 * 1024 * 1024;
const MAX_TREE_ENTRIES = 50_000;
const MAX_SOURCE_FILES = 128;
const MAX_SOURCE_FILE_BYTES = 1024 * 1024;
const CLASS_DECLARATIONS = new Set(["class", "object", "interface"]);

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
      if (value === "}" && parens === 0 && brackets === 0 && braces === 0 && angles === 0 && (pairs.closeToOpen.get(bodyOpen) ?? -1) < index) break;
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
    const supertypeNames = supertypeStart === -1
      ? []
      : splitTopLevel(tokens, index + supertypeStart, bodyOpen)
        .map(([start, end]) => {
          let typeEnd = start;
          while (typeEnd < end && !["(", "<", "by"].includes(tokens[typeEnd].value)) typeEnd += 1;
          return tokenText(source, tokens, start, typeEnd).trim();
        })
        .filter(Boolean);
    const implementsInterface = supertypeNames.some((name) => name.split(".").at(-1) === "MediaServerClient");
    if (tokens[bodyOpen]?.value !== "{") {
      if (implementsInterface) {
        throw new ContractError(`${path}:${tokens[index].line}: MediaServerClient adapter ${nameToken.value} has no body`);
      }
      classes.push({
        name: nameToken.value,
        path,
        implementsInterface,
        properties: {},
        supertypeNames,
        methods: [],
        startIndex: index,
        bodyOpen,
        bodyClose: null,
        headerText: tokenText(source, tokens, index, bodyOpen),
      });
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
      supertypeNames,
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

function httpClientReceiverNames(classDeclaration) {
  return new Set(
    Object.entries(classDeclaration?.properties ?? {})
      .filter(([, type]) => type.replace(/\?$/, "").split(".").at(-1) === "HttpClient")
      .map(([name]) => name),
  );
}
function endpointContextForClass(classDeclaration, context) {
  const httpClientReceivers = httpClientReceiverNames(classDeclaration);
  return httpClientReceivers.size > 0 ? { ...context, httpClientReceivers } : context;
}
function komgaClientType(classDeclaration) {
  return classDeclaration?.supertypeNames
    .map((name) => name.split(".").at(-1))
    .find((name) => name.startsWith("Komga") && name.endsWith("Client")) ?? null;
}

function directStringEndpoints(source, tokens, pairs, startIndex, endIndex, context = {}) {
  const endpoints = [];
  const clientVerbs = new Map([
    ["get", "GET"], ["post", "POST"], ["put", "PUT"], ["patch", "PATCH"],
    ["delete", "DELETE"], ["request", "REQUEST"], ["preparePost", "POST"], ["sseSession", "SSE"],
  ]);
  const serverVerbs = new Map([
    ...clientVerbs,
    ["head", "HEAD"], ["options", "OPTIONS"], ["sse", "SSE"],
  ]);
  const routeNames = new Set(["route", "path", "url", "sse"]);
  const httpClientReceivers = context.httpClientReceivers;
  const clientMode = httpClientReceivers instanceof Set;
  for (let index = startIndex; index < endIndex; index += 1) {
    if (tokens[index].kind !== "identifier") continue;
    const name = tokens[index].value;
    const receiver = receiverFor(tokens, pairs, index);
    if (
      clientMode &&
      name === "sseSession" &&
      httpClientReceivers.has(receiver) &&
      tokens[index + 1]?.value === "{"
    ) {
      const bodyClose = pairs.openToClose.get(index + 1);
      if (bodyClose == null || bodyClose >= endIndex) continue;
      const nested = directStringEndpoints(source, tokens, pairs, index + 2, bodyClose, {
        ...context,
        clientRequestDepth: (context.clientRequestDepth ?? 0) + 1,
      });
      const nestedUrl = nested.find((endpoint) => endpoint.verb === "URL");
      if (nestedUrl) {
        endpoints.push({
          verb: "URL",
          path: nestedUrl.path,
          literal: nestedUrl.literal,
          sourcePath: context.sourcePath,
          line: tokens[index].line,
          function: context.functionName ?? null,
        });
      }
      continue;
    }
    const call = callAt(tokens, pairs, index);
    const clientRequest = clientMode && clientVerbs.has(name) && httpClientReceivers.has(receiver);
    if (!call && !(clientRequest && tokens[index + 1]?.value === "{")) continue;
    const ranges = call?.ranges ?? [];
    const first = ranges.length > 0 ? valueFromRange(tokens, ranges[0]) : null;
    const requestVerb = clientMode ? clientVerbs.get(name) : serverVerbs.get(name);
    const clientPathContext = clientMode && (context.clientRequestDepth ?? 0) > 0;
    if (
      first != null &&
      (clientRequest || (!clientMode && (serverVerbs.has(name) || routeNames.has(name))) ||
        (clientPathContext && ["path", "url"].includes(name)))
    ) {
      const verb = clientRequest
        ? requestVerb
        : name === "route" ? "ROUTE"
          : name === "path" ? "PATH"
            : name === "url" ? "URL"
              : requestVerb;
      endpoints.push({
        verb,
        path: normalizePath(first),
        literal: first,
        sourcePath: context.sourcePath,
        line: tokens[index].line,
        function: context.functionName ?? null,
      });
    }
    if (
      name === "appendPathSegments" &&
      call.ranges.length > 0 &&
      (!clientMode || clientPathContext)
    ) {
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
    if (
      (clientRequest || (!clientMode && serverVerbs.has(name))) &&
      tokens[index + 1]?.value === "{"
    ) {
      const bodyClose = pairs.openToClose.get(index + 1);
      if (bodyClose == null || bodyClose >= endIndex) continue;
      const nested = directStringEndpoints(source, tokens, pairs, index + 2, bodyClose, {
        ...context,
        clientRequestDepth: clientMode ? (context.clientRequestDepth ?? 0) + 1 : context.clientRequestDepth,
      });
      const nestedPath = nested.find((endpoint) => ["PATH", "URL"].includes(endpoint.verb));
      if (nestedPath) {
        endpoints.push({
          verb: requestVerb,
          path: nestedPath.path,
          literal: nestedPath.literal,
          sourcePath: context.sourcePath,
          line: tokens[index].line,
          function: context.functionName ?? null,
        });
      }
    }
    if (!clientMode && name === "sseSession" && tokens[index + 1]?.value === "{") {
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

function endpointsForSource(parsed, { includeClientMetadata = false } = {}) {
  const endpoints = [];
  const context = { sourcePath: parsed.entry.path };
  for (const classDeclaration of parsed.classes) {
    const methodContext = endpointContextForClass(classDeclaration, context);
    const receiverType = includeClientMetadata ? komgaClientType(classDeclaration) : null;
    for (const method of classDeclaration.methods) {
      const methodEndpoints = endpointsForFunction(parsed.text, parsed.tokens, parsed.pairs, method, methodContext);
      endpoints.push(...methodEndpoints.map((endpoint) => includeClientMetadata ? ({
        ...endpoint,
        argumentCount: method.parameters.length,
        parameterTypes: method.parameters.map((parameter) => parameter.normalizedType),
        receiverType,
      }) : endpoint));
    }
  }
  const classRanges = parsed.classes.map((item) => [item.bodyOpen, item.bodyClose]);
  for (let index = 0; index < parsed.tokens.length; index += 1) {
    if (parsed.tokens[index].value !== "fun") continue;
    if (classRanges.some(([start, end]) => index > start && index < end)) continue;
    const method = parseFunction(parsed.text, parsed.tokens, parsed.pairs, index, parsed.tokens.length);
    if (method.body) {
      const methodEndpoints = endpointsForFunction(parsed.text, parsed.tokens, parsed.pairs, method, context);
      endpoints.push(...methodEndpoints.map((endpoint) => includeClientMetadata ? ({
        ...endpoint,
        argumentCount: method.parameters.length,
        parameterTypes: method.parameters.map((parameter) => parameter.normalizedType),
        receiverType: null,
      }) : endpoint));
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
          const resolved = !externalKomga && receiverType
            ? findClassMethod(allClasses, receiverType, call.name, call.args.length)
            : null;
          let endpoints = [];
          if (resolved) {
            endpoints = endpointsForFunction(
              resolved.classDeclaration.source,
              resolved.classDeclaration._tokens,
              resolved.classDeclaration._pairs,
              resolved.method,
              endpointContextForClass(resolved.classDeclaration, { sourcePath: resolved.classDeclaration.path }),
            );
          }
          const resolution = externalKomga
            ? "unresolved-external-komga-client"
            : resolved ? "resolved-source" : "non-http";
          const reason = !externalKomga && !resolved
            ? receiverType ? "no-matching-local-method" : "untyped-helper-or-mapper-call"
            : undefined;
          return {
            argumentCount: call.args.length,
            endpoints,
            expression: call.expression,
            name: call.name,
            receiver: call.receiver,
            receiverType,
            resolution,
            ...(reason ? { reason } : {}),
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
function routeCandidateView(route) {
  return {
    argumentCount: route.argumentCount,
    function: route.function,
    parameterTypes: route.parameterTypes,
    path: route.path,
    sourcePath: route.sourcePath,
    verb: route.verb,
  };
}

function resolveKomgaClientCall(call, routes) {
  if (!call.receiverType) {
    return { resolution: "unresolved-external-komga-client", reason: "missing-receiver-type", endpoints: [] };
  }
  const namedRoutes = routes.filter((route) =>
    route.function === call.name &&
    ["DELETE", "GET", "PATCH", "POST", "PUT", "REQUEST"].includes(route.verb),
  );
  const receiverRoutes = namedRoutes.filter((route) => route.receiverType === call.receiverType);
  if (receiverRoutes.length === 0) {
    return {
      resolution: "unresolved-external-komga-client",
      reason: namedRoutes.length > 0 ? "no-route-for-receiver-type" : "no-route-for-method",
      endpoints: [],
      candidates: namedRoutes.map(routeCandidateView),
    };
  }
  let matchingRoutes = receiverRoutes;
  if (receiverRoutes.length > 1) {
    matchingRoutes = receiverRoutes.filter((route) => route.argumentCount === call.argumentCount);
    if (matchingRoutes.length !== 1) {
      return {
        resolution: "unresolved-external-komga-client",
        reason: matchingRoutes.length > 1 ? "ambiguous-overload-match" : "no-overload-matches-argument-count",
        endpoints: [],
        candidates: (matchingRoutes.length > 1 ? matchingRoutes : receiverRoutes).map(routeCandidateView),
      };
    }
  }
  const [route] = matchingRoutes;
  return {
    resolution: "resolved-external-komga-client",
    endpoints: [{
      function: route.function,
      line: route.line,
      path: route.path,
      sourcePath: route.sourcePath,
      verb: route.verb,
    }],
    sourcePath: route.sourcePath,
  };
}

function joinExternalKomgaRoutes(adapters, routes) {
  for (const adapter of adapters) {
    if (adapter.library !== "komga") continue;
    for (const operation of adapter.operations) {
      for (const call of operation.clientCalls) {
        if (call.resolution !== "unresolved-external-komga-client") continue;
        const result = resolveKomgaClientCall(call, routes);
        call.resolution = result.resolution;
        call.endpoints = result.endpoints;
        call.sourcePath = result.sourcePath ?? null;
        delete call.reason;
        delete call.candidates;
        if (result.reason) call.reason = result.reason;
        if (result.candidates) call.candidates = result.candidates;
      }
    }
  }
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

async function extractContract({ komfEntry, komeliaEntry, ref, expectedVersion }) {
  const komfRepository = await fetchRepository(KOMF_REPO, ref, (entry) =>
    (entry.path.startsWith("komf-mediaserver/src/") && /\/[^/]*Main\/kotlin\//.test(entry.path)) ||
    (entry.path.startsWith("komf-app/src/main/kotlin/") && /\/(?:[^/]*Routes|ServerModule)\.kt$/.test(entry.path)) ||
    entry.path.startsWith(KOMF_API_MODEL_PREFIX) ||
    entry.path.startsWith("komf-app/src/main/kotlin/snd/komf/app/api/deprecated/dto/") ||
    entry.path === "komf-core/src/commonMain/kotlin/snd/komf/model/SeriesSearchResult.kt" ||
    entry.path === "komf-core/src/commonMain/kotlin/snd/komf/providers/CoreProviders.kt",
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
  const komfClientRef = KOMF_CLIENT_RELEASE.ref;
  const komfClientRepository = await fetchRepository(KOMF_CLIENT_REPO, komfClientRef, (entry) =>
    entry.path.startsWith(KOMF_CLIENT_SOURCE_PREFIX) || entry.path.startsWith(KOMF_API_MODEL_PREFIX),
  );
  const komfClientCatalogEntry = komfClientRepository.tree.tree.find((entry) => entry.path === "gradle/libs.versions.toml");
  if (!komfClientCatalogEntry) throw new ContractError(`${KOMF_CLIENT_REPO}@${komfClientRef}: gradle/libs.versions.toml not found`);
  const komfClientCatalogSource = await komfClientRepository.get(komfClientCatalogEntry);
  const komfClientVersion = parseVersion(komfClientCatalogSource, komfClientCatalogEntry.path, "app-version");
  if (komfClientVersion !== KOMF_CLIENT_RELEASE.version) {
    throw new ContractError(`${KOMF_CLIENT_REPO}@${komfClientRef}: expected Komf client ${KOMF_CLIENT_RELEASE.version}, source reports ${komfClientVersion}`);
  }
  const komfClientParsed = [];
  for (const entry of komfClientRepository.entries) {
    const text = await komfClientRepository.get(entry);
    komfClientParsed.push(parseSource(KOMF_CLIENT_REPO, komfClientRef, entry, text));
  }
  const clientOperations = extractKomfClientOperations(komfClientParsed);

  const komeliaRef = komeliaEntry.baseline.value;
  const komeliaCatalogEntry = await repositoryFile(KOMELIA_REPO, komeliaRef, "gradle/libs.versions.toml");
  const komeliaCatalogSource = await sourceText(KOMELIA_REPO, komeliaRef, komeliaCatalogEntry);
  const komeliaVersion = parseVersion(komeliaCatalogSource, komeliaCatalogEntry.path, "app-version");
  const bundledClientVersion = parseVersion(komeliaCatalogSource, komeliaCatalogEntry.path, "komf-client");
  if (komeliaVersion !== "0.19.3" || bundledClientVersion !== komfClientVersion) {
    throw new ContractError(`${KOMELIA_REPO}@${komeliaRef}: expected Komelia 0.19.3 with komf-client ${komfClientVersion}, source reports ${komeliaVersion} with ${bundledClientVersion}`);
  }
  const komeliaRepository = await fetchRepositorySubtrees(KOMELIA_REPO, komeliaRef, KOMELIA_SOURCE_PREFIXES, () => true);
  const komeliaParsed = [];
  for (const entry of komeliaRepository.entries) {
    const text = await komeliaRepository.get(entry);
    komeliaParsed.push(parseSource(KOMELIA_REPO, komeliaRef, entry, text));
  }
  attachKomeliaCallSites(clientOperations, komeliaParsed);
  const komeliaFactory = komeliaParsed.find((parsed) => parsed.entry.path.endsWith("/KomfViewModelFactory.kt"));
  if (!komeliaFactory) throw new ContractError(`${KOMELIA_REPO}@${komeliaRef}: KomfViewModelFactory.kt was not selected`);
  const komeliaCallSitePaths = new Set([
    komeliaFactory.entry.path,
    ...clientOperations.flatMap((operation) => operation.callSites.map((callSite) => callSite.sourcePath)),
  ]);
  const komeliaSourceRecords = komeliaParsed
    .filter((parsed) => komeliaCallSitePaths.has(parsed.entry.path))
    .map((parsed) => sourceRecord(KOMELIA_REPO, komeliaRef, parsed.entry, parsed.text));
  const komeliaCatalogRecord = sourceRecord(KOMELIA_REPO, komeliaRef, komeliaCatalogEntry, komeliaCatalogSource);
  const komfClientSourceRecords = [
    ...komfClientParsed.map((parsed) => sourceRecord(KOMF_CLIENT_REPO, komfClientRef, parsed.entry, parsed.text)),
    sourceRecord(KOMF_CLIENT_REPO, komfClientRef, komfClientCatalogEntry, komfClientCatalogSource),
  ];
  const appDtoSources = parsedSources.filter((parsed) =>
    parsed.entry.path.startsWith(KOMF_API_MODEL_PREFIX) ||
    parsed.entry.path.startsWith("komf-app/src/main/kotlin/snd/komf/app/api/deprecated/dto/") ||
    parsed.entry.path === "komf-core/src/commonMain/kotlin/snd/komf/model/SeriesSearchResult.kt" ||
    parsed.entry.path === "komf-core/src/commonMain/kotlin/snd/komf/providers/CoreProviders.kt",
  );
  const appDtoTypes = parseDtoTypes(appDtoSources);
  const clientDtoTypes = parseDtoTypes(komfClientParsed.filter((parsed) => parsed.entry.path.startsWith(KOMF_API_MODEL_PREFIX)));
  const appHttpApi = buildKomfHttpApi({
    appRoutes,
    parsedSources: [...parsedSources, ...komfClientParsed],
    apiTypes: appDtoTypes,
    clientOperations,
    clientSourceRecords: komfClientSourceRecords,
    komeliaSourceRecords,
    komeliaVersion,
    komeliaRef,
    komfClientVersion,
    komfClientRef,
    komeliaCatalogRecord,
  });

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
    return endpointsForSource(parsed, { includeClientMetadata: true }).map((route) => ({
      argumentCount: route.argumentCount,
      function: route.function,
      line: route.line,
      parameterTypes: route.parameterTypes,
      path: route.path,
      receiverType: route.receiverType,
      sourcePath: route.sourcePath,
      verb: route.verb,
    }));
  });
  const externalSourceRecords = externalParsed.map((parsed) => sourceRecord(KOMGA_CLIENT_REPO, externalRef, parsed.entry, parsed.text));
  const ssePaths = [...new Set(externalRoutes
    .filter((route) => route.function === "sseSession" && ["SSE", "URL"].includes(route.verb))
    .map((route) => route.path))].sort();
  joinExternalKomgaRoutes(adapters, externalRoutes);
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
      httpApi: appHttpApi,
      routePrefixes,
      sourcePaths: appSources.map((item) => item.path).sort(),
    },
    dependencies: {
      komfClient: {
        commit: komfClientRef,
        dtoTypes: clientDtoTypes,
        repository: KOMF_CLIENT_REPO,
        routes: appHttpApi.clientOperations,
        sourceHashes: komfClientSourceRecords,
        version: komfClientVersion,
      },
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

async function repositoryTreeAtPath(repo, ref, pathParts) {
  let tree = await githubJson(`repos/${repo}/git/trees/${ref}`);
  for (const part of pathParts) {
    const directory = tree.tree?.find((entry) => entry.type === "tree" && entry.path === part);
    if (!directory) throw new ContractError(`${repo}@${ref}: repository path component ${part} was not found`);
    tree = await githubJson(`repos/${repo}/git/trees/${directory.sha}`);
    if (!Array.isArray(tree.tree) || tree.tree.length > MAX_TREE_ENTRIES) {
      throw new ContractError(`${repo}@${ref}:${pathParts.join("/")}: invalid or oversized tree`);
    }
  }
  return tree;
}

async function repositoryFile(repo, ref, path) {
  const parts = path.split("/");
  const parent = await repositoryTreeAtPath(repo, ref, parts.slice(0, -1));
  const entry = parent.tree.find((candidate) => candidate.type === "blob" && candidate.path === parts.at(-1));
  if (!entry) throw new ContractError(`${repo}@${ref}:${path}: source file was not found`);
  return { ...entry, path };
}

async function fetchRepositorySubtrees(repo, ref, prefixes, includeEntry) {
  const selected = new Map();
  for (const prefix of prefixes) {
    const directory = await repositoryTreeAtPath(repo, ref, prefix.split("/"));
    const tree = await githubJson(`repos/${repo}/git/trees/${directory.sha}?recursive=1`);
    if (tree.truncated) throw new ContractError(`${repo}@${ref}:${prefix}: GitHub subtree response was truncated`);
    if (!Array.isArray(tree.tree) || tree.tree.length > MAX_TREE_ENTRIES) {
      throw new ContractError(`${repo}@${ref}:${prefix}: GitHub subtree is invalid or oversized`);
    }
    for (const entry of tree.tree) {
      if (entry.type !== "blob") continue;
      const fullEntry = { ...entry, path: `${prefix}/${entry.path}` };
      if (!fullEntry.path.endsWith(".kt") || !includeEntry(fullEntry)) continue;
      const previous = selected.get(fullEntry.path);
      if (previous && previous.sha !== fullEntry.sha) {
        throw new ContractError(`${repo}@${ref}:${fullEntry.path}: overlapping source selectors disagree`);
      }
      selected.set(fullEntry.path, fullEntry);
    }
  }
  const entries = [...selected.values()].sort((left, right) => compareText(left.path, right.path));
  if (entries.length === 0) throw new ContractError(`${repo}@${ref}: no Kotlin sources selected`);
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
  return { entries, get };
}

function annotationTextForDeclaration(source, lineNumber) {
  const lines = source.split("\n");
  const annotations = [];
  for (let index = lineNumber - 2; index >= 0; index -= 1) {
    const line = lines[index].trim();
    if (!line) continue;
    if (!line.startsWith("@")) break;
    annotations.unshift(line);
  }
  return annotations.join("\n");
}

function parseDtoFields(source, tokens, pairs, openIndex, closeIndex) {
  return splitTopLevel(tokens, openIndex + 1, closeIndex, { typeArguments: true })
    .map(([rawStart, rawEnd]) => {
      let start = rawStart;
      let end = rawEnd;
      while (start < end && tokens[start].value === ",") start += 1;
      while (end > start && tokens[end - 1].value === ",") end -= 1;
      if (start >= end) return null;

      let colon = -1;
      let equals = -1;
      let depth = 0;
      for (let index = start; index < end; index += 1) {
        const value = tokens[index].value;
        if (["(", "[", "{", "<"].includes(value)) depth += 1;
        else if ([")", "]", "}", ">"].includes(value)) depth -= 1;
        else if (depth === 0 && value === ":" && colon === -1) colon = index;
        else if (depth === 0 && value === "=" && equals === -1) equals = index;
      }
      if (colon === -1) return null;
      const nameToken = tokens[colon - 1];
      if (!nameToken || nameToken.kind !== "identifier") return null;
      const typeEnd = equals === -1 ? end : equals;
      const type = tokenText(source, tokens, colon + 1, typeEnd);
      const nullable = type.trim().endsWith("?");
      let serialName = null;
      for (let index = start; index < colon; index += 1) {
        if (tokens[index].value !== "SerialName" || tokens[index + 1]?.value !== "(") continue;
        const argument = tokens[index + 2];
        if (argument?.kind === "string") serialName = argument.value;
      }
      return {
        name: nameToken.value,
        type,
        nullable,
        required: !nullable && equals === -1,
        hasDefault: equals !== -1,
        ...(serialName ? { serialName } : {}),
      };
    })
    .filter(Boolean);
}

function primaryConstructorRange(parsed, declaration) {
  let angleDepth = 0;
  for (let index = declaration.startIndex + 2; index < declaration.bodyOpen; index += 1) {
    const value = parsed.tokens[index].value;
    if (value === "<") angleDepth += 1;
    else if (value === ">" && angleDepth > 0) angleDepth -= 1;
    else if (value === ":" && angleDepth === 0) return null;
    else if (value === "(" && angleDepth === 0) {
      const closeIndex = parsed.pairs.openToClose.get(index);
      if (closeIndex != null && closeIndex < declaration.bodyOpen) return { openIndex: index, closeIndex };
    }
  }
  return null;
}

function parseDtoTypes(parsedSources) {
  const types = [];
  for (const parsed of parsedSources) {
    for (const declaration of parsed.classes) {
      const declarationIndex = declaration.startIndex;
      const annotations = annotationTextForDeclaration(parsed.text, parsed.tokens[declarationIndex].line);
      const serializable = /@Serializable\b/.test(annotations);
      const modifiers = parsed.tokens
        .slice(Math.max(0, declarationIndex - 3), declarationIndex)
        .map((token) => token.value);
      const declarationKeyword = parsed.tokens[declarationIndex].value;
      const kind = modifiers.includes("enum") ? "enum"
        : declarationKeyword === "object" && modifiers.includes("data") ? "dataObject"
          : modifiers.includes("data") ? "dataClass"
            : modifiers.includes("value") ? "valueClass"
              : modifiers.includes("sealed") ? "sealed"
                : declarationKeyword;
      const isProviderVariant = declaration.supertypeNames.some((name) => name.split(".").at(-1) === "KomfProviders");
      if (!serializable && kind !== "enum" && !isProviderVariant) continue;
      const constructor = primaryConstructorRange(parsed, declaration);
      const constructorFields = constructor
        ? parseDtoFields(parsed.text, parsed.tokens, parsed.pairs, constructor.openIndex, constructor.closeIndex)
        : [];
      const bodyText = declaration.bodyClose == null || parsed.tokens[declaration.bodyOpen]?.value !== "{"
        ? ""
        : parsed.text.slice(parsed.tokens[declaration.bodyOpen].end, parsed.tokens[declaration.bodyClose].start);
      const enumValues = kind === "enum"
        ? bodyText.split(";")[0].split(",").map((part) =>
          part.trim().replace(/^(?:@[A-Za-z_][A-Za-z0-9_.]*(?:\([^)]*\))?\s*)+/, "").match(/^([A-Za-z_][A-Za-z0-9_]*)/)?.[1],
        ).filter(Boolean)
        : [];
      const serialName = annotations.match(/@SerialName\s*\(\s*"([^"]+)"/)?.[1] ?? null;
      const customSerializer = annotations.match(/@Serializable\s*\(\s*(?:with\s*=\s*)?([A-Za-z_][A-Za-z0-9_.]*)::class/)?.[1] ?? null;
      types.push({
        name: declaration.name,
        kind,
        serializable,
        ...(customSerializer ? { customSerializer } : {}),
        ...(serialName ? { serialName } : {}),
        fields: constructorFields,
        ...(enumValues.length > 0 ? { enumValues } : {}),
        sourcePath: parsed.entry.path,
        line: parsed.tokens[declarationIndex].line,
      });
    }
  }
  return types.sort((left, right) => compareText(`${left.sourcePath}:${left.line}:${left.name}`, `${right.sourcePath}:${right.line}:${right.name}`));
}

function normalizeClientPath(path, className) {
  let normalized = path;
  if (className === "KomfMetadataClient") {
    normalized = normalized.replaceAll("$metadataApiPrefix", "/api/{mediaServer}/metadata");
  }
  if (className === "KomfMediaServerClient") {
    normalized = normalized.replaceAll("$mediaServerApiPrefix", "/api/{mediaServer}/media-server");
  }
  normalized = normalized
    .replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)(?:\.value)?\}/g, "{$1}")
    .replace(/\$([A-Za-z_][A-Za-z0-9_]*)(?:\.value)?/g, "{$1}");
  return normalizePath(normalized);
}

function extractKomfClientOperations(parsedSources) {
  const operations = [];
  for (const parsed of parsedSources.filter((item) => item.entry.path.startsWith(KOMF_CLIENT_SOURCE_PREFIX))) {
    const endpoints = endpointsForSource(parsed);
    for (const endpoint of endpoints) {
      if (!endpoint.function || !["GET", "POST", "PUT", "PATCH", "DELETE", "SSE"].includes(endpoint.verb)) continue;
      const classDeclaration = parsed.classes.find((candidate) => candidate.methods.some((method) => method.name === endpoint.function));
      const method = classDeclaration?.methods.find((candidate) => candidate.name === endpoint.function);
      if (!classDeclaration || !method) continue;
      const bodyParameterName = method.body?.text.match(/\bsetBody\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)/)?.[1] ?? null;
      const bodyParameter = bodyParameterName
        ? method.parameters.find((parameter) => parameter.name === bodyParameterName)
        : null;
      operations.push({
        clientClass: classDeclaration.name,
        function: method.name,
        line: method.line,
        route: {
          method: endpoint.verb === "SSE" ? "GET" : endpoint.verb,
          transport: endpoint.verb === "SSE" ? "sse" : "http",
          path: normalizeClientPath(endpoint.path, classDeclaration.name),
          sourceLine: endpoint.line,
        },
        request: {
          body: bodyParameter ? { name: bodyParameter.name, type: bodyParameter.type } : null,
          parameters: method.parameters
            .filter((parameter) => parameter.name !== bodyParameterName)
            .map(({ name, type }) => ({ name, type })),
        },
        responseType: method.returnType,
        sourcePath: parsed.entry.path,
        callSites: [],
      });
    }
  }
  operations.sort((left, right) => compareText(`${left.sourcePath}:${left.line}:${left.function}`, `${right.sourcePath}:${right.line}:${right.function}`));
  const operationKeys = new Set();
  for (const operation of operations) {
    const key = `${operation.clientClass}.${operation.function}`;
    if (operationKeys.has(key)) throw new ContractError(`Komf client operation ${key} has multiple endpoint definitions`);
    operationKeys.add(key);
  }
  return operations;
}

function attachKomeliaCallSites(operations, parsedSources) {
  const byClassAndMethod = new Map(operations.map((operation) => [`${operation.clientClass}.${operation.function}`, operation]));
  const clientClassNames = new Set(operations.map((operation) => operation.clientClass));
  for (const parsed of parsedSources) {
    for (const classDeclaration of parsed.classes) {
      const clientProperties = new Map(
        Object.entries(classDeclaration.properties)
          .filter(([, type]) => clientClassNames.has(type.replace(/\?$/, "")))
          .map(([name, type]) => [name, type.replace(/\?$/, "")]),
      );
      for (const method of classDeclaration.methods) {
        if (!method.body) continue;
        for (let index = method.body.startIndex; index < method.body.endIndex; index += 1) {
          const candidateOperations = operations.filter((operation) => operation.function === parsed.tokens[index].value);
          if (candidateOperations.length === 0 || parsed.tokens[index + 1]?.value !== "(") continue;
          const receiver = receiverFor(parsed.tokens, parsed.pairs, index) ?? "";
          if (!receiver) continue;
          const receiverName = receiver.split(".").at(-1);
          const clientClass = clientProperties.get(receiverName) ?? null;
          if (!clientClass) continue;
          const operation = byClassAndMethod.get(`${clientClass}.${parsed.tokens[index].value}`);
          if (!operation) continue;
          operation.callSites.push({
            callerClass: classDeclaration.name,
            callerFunction: method.name,
            line: parsed.tokens[index].line,
            sourcePath: parsed.entry.path,
          });
        }
      }
    }
  }
  for (const operation of operations) {
    operation.callSites.sort((left, right) => compareText(`${left.sourcePath}:${left.line}`, `${right.sourcePath}:${right.line}`));
  }
}

function pathMatches(pattern, path) {
  const regex = pattern.split(/(\{[^}]+\})/)
    .map((part) => part.startsWith("{") && part.endsWith("}")
      ? "[^/]+"
      : part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("");
  return new RegExp(`^${regex}$`).test(path);
}

function routeMounts(className) {
  switch (className) {
    case "ConfigRoutes":
    case "JobRoutes":
    case "NotificationRoutes":
      return [{ prefix: "/api", mediaServer: null }];
    case "MediaServerRoutes":
    case "MetadataRoutes":
      return ["komga", "kavita"].map((mediaServer) => ({ prefix: `/api/${mediaServer}`, mediaServer }));
    case "DeprecatedConfigRoutes":
      return [{ prefix: "", mediaServer: null }];
    case "DeprecatedMetadataRoutes":
      return ["komga", "kavita"].map((mediaServer) => ({ prefix: `/${mediaServer}`, mediaServer }));
    default:
      throw new ContractError(`No Komf app route mount is defined for ${className}`);
  }
}

function routeLocalPrefix(className, route) {
  switch (className) {
    case "JobRoutes": return "/jobs";
    case "MediaServerRoutes": return "/media-server";
    case "MetadataRoutes": return "/metadata";
    case "NotificationRoutes":
      if (route.function.startsWith("discord")) return "/notifications/discord";
      if (route.function.startsWith("apprise")) return "/notifications/apprise";
      throw new ContractError(`${route.sourcePath}:${route.line}: notification route has no service prefix`);
    case "ConfigRoutes":
    case "DeprecatedConfigRoutes":
    case "DeprecatedMetadataRoutes":
      return "";
    default:
      throw new ContractError(`No Komf app route prefix is defined for ${className}`);
  }
}

function pathJoin(...parts) {
  return normalizePath(parts.filter(Boolean).join("/"));
}

function appRouteContract(className, functionName) {
  const json = (bodyType, status = 200) => ({ status, contentType: "application/json", bodyType });
  const empty = (status, contentType = null) => ({ status, contentType, bodyType: null });
  const error = (status, bodyType = "KomfErrorResponse", condition = null) => ({
    status,
    contentType: bodyType ? "application/json" : null,
    bodyType,
    ...(condition ? { condition } : {}),
  });
  const request = (bodyType) => ({ bodyType, contentType: "application/json" });
  const key = `${className}.${functionName}`;
  switch (key) {
    case "ConfigRoutes.getConfigRoute":
      return { requestBody: null, queryParameters: [], responses: [json("KomfConfig")] };
    case "ConfigRoutes.updateConfigRoute":
      return {
        requestBody: request("KomfConfigUpdateRequest"),
        queryParameters: [],
        responses: [empty(204), error(422, "KomfErrorResponse", "configuration persistence callback throws")],
      };
    case "ConfigRoutes.updateMangaBakaDB":
    case "ConfigRoutes.updateBookWalkerDb":
      return {
        requestBody: null,
        queryParameters: [],
        responses: [{
          status: 200,
          contentType: "application/jsonl",
          bodyType: "DownloadProgress",
          framing: "one JSON object per newline; ProgressEvent continues, FinishedEvent/ErrorEvent ends",
        }],
      };
    case "JobRoutes.getJobsRoute":
      return {
        requestBody: null,
        queryParameters: [
          { name: "status", type: "KomfMetadataJobStatus", required: false },
          { name: "page", type: "Int", required: false, default: 0 },
          { name: "pageSize", type: "Int", required: false, default: 1000 },
        ],
        responses: [json("KomfPage<List<KomfMetadataJob>>"), error(400, null, "invalid status, page, or pageSize yields an empty body")],
      };
    case "JobRoutes.getJobRoute":
      return {
        requestBody: null,
        queryParameters: [],
        responses: [json("KomfMetadataJob"), error(404, null, "job id is not present"), error(400, "KomfErrorResponse", "jobId is not a UUID")],
      };
    case "JobRoutes.metadataEventFlowRoute":
      return {
        requestBody: null,
        queryParameters: [],
        responses: [{
          status: 200,
          contentType: "text/event-stream",
          bodyType: "KomfMetadataJobEvent",
          framing: "Server-Sent Events",
        }, error(400, "KomfErrorResponse", "jobId is not a UUID")],
      };
    case "JobRoutes.deleteAllRoute":
      return { requestBody: null, queryParameters: [], responses: [empty(204)] };
    case "MediaServerRoutes.checkConnectionRoute":
      return {
        requestBody: null,
        queryParameters: [],
        responses: [{
          ...json("KomfMediaServerConnectionResponse"),
          note: "HTTP 200 for success and connection failure; failure is represented by success=false and error fields",
        }],
      };
    case "MediaServerRoutes.getLibrariesRoute":
      return { requestBody: null, queryParameters: [], responses: [json("List<KomfMediaServerLibrary>")] };
    case "MetadataRoutes.getProvidersRoute":
      return {
        requestBody: null,
        queryParameters: [{ name: "libraryId", type: "String", required: false }],
        responses: [json("List<String>")],
      };
    case "MetadataRoutes.searchSeriesRoute":
      return {
        requestBody: null,
        queryParameters: [
          { name: "name", type: "String", required: true },
          { name: "seriesId", type: "String", required: false },
          { name: "libraryId", type: "String", required: false },
        ],
        responses: [
          json("List<KomfMetadataSeriesSearchResult>"),
          error(400, null, "missing name query parameter"),
          error(500, "KomfErrorResponse", "unexpected exception"),
          error("upstream-status", "KomfErrorResponse", "upstream ResponseException; preserves upstream status"),
        ],
      };
    case "MetadataRoutes.getSeriesCoverRoute":
      return {
        requestBody: null,
        queryParameters: [
          { name: "libraryId", type: "String", required: true },
          { name: "provider", type: "CoreProviders", required: true },
          { name: "providerSeriesId", type: "String", required: true },
        ],
        responses: [
          { status: 200, contentType: "application/octet-stream", bodyType: "ByteArray" },
          error(404, null, "series cover is absent"),
          error(400, "KomfErrorResponse", "required query parameter missing or provider enum invalid"),
        ],
      };
    case "MetadataRoutes.identifySeriesRoute":
      return { requestBody: request("KomfIdentifyRequest"), queryParameters: [], responses: [json("KomfMetadataJobResponse")] };
    case "MetadataRoutes.matchSeriesRoute":
      return { requestBody: null, queryParameters: [], responses: [json("KomfMetadataJobResponse")] };
    case "MetadataRoutes.matchLibraryRoute":
      return { requestBody: null, queryParameters: [], responses: [empty(202)] };
    case "MetadataRoutes.resetSeriesRoute":
      return {
        requestBody: null,
        queryParameters: [{ name: "removeComicInfo", type: "Boolean", required: false, default: false }],
        responses: [
          empty(204),
          error(422, "KomfErrorResponse", "ComicInfoException"),
          error(400, "KomfErrorResponse", "invalid path or server argument"),
        ],
      };
    case "MetadataRoutes.resetLibraryRoute":
      return {
        requestBody: null,
        queryParameters: [{ name: "removeComicInfo", type: "Boolean", required: false, default: false }],
        responses: [empty(204)],
      };
    case "NotificationRoutes.discordGetTemplatesRoute":
      return { requestBody: null, queryParameters: [], responses: [json("KomfDiscordTemplates")] };
    case "NotificationRoutes.discordUpdateTemplatesRoute":
      return {
        requestBody: request("KomfDiscordTemplates"),
        queryParameters: [],
        responses: [
          json("KomfDiscordTemplates"),
          error(422, "KomfErrorResponse", "template update exception; handler subsequently attempts its unconditional 200 response"),
        ],
      };
    case "NotificationRoutes.discordSendRoute":
      return {
        requestBody: request("KomfDiscordRequest"),
        queryParameters: [],
        responses: [
          empty(200, "text/plain"),
          error("upstream-status", "KomfErrorResponse", "upstream ResponseException; handler subsequently attempts its unconditional 200 response"),
        ],
      };
    case "NotificationRoutes.discordRenderRoute":
      return { requestBody: request("KomfDiscordRequest"), queryParameters: [], responses: [json("KomfDiscordRenderResult")] };
    case "NotificationRoutes.appriseGetTemplatesRoute":
      return { requestBody: null, queryParameters: [], responses: [json("KomfAppriseTemplates")] };
    case "NotificationRoutes.appriseUpdateTemplatesRoute":
      return {
        requestBody: request("KomfAppriseTemplates"),
        queryParameters: [],
        responses: [
          json("KomfAppriseTemplates"),
          error(422, "KomfErrorResponse", "template update exception; handler subsequently attempts its unconditional 200 response"),
        ],
      };
    case "NotificationRoutes.appriseSendRoute":
      return {
        requestBody: request("KomfAppriseRequest"),
        queryParameters: [],
        responses: [
          empty(200, "text/plain"),
          error(422, "KomfErrorResponse", "send exception; handler subsequently attempts its unconditional 200 response"),
        ],
      };
    case "NotificationRoutes.appriseRenderRoute":
      return { requestBody: request("KomfAppriseRequest"), queryParameters: [], responses: [json("KomfAppriseRenderResult")] };
    case "DeprecatedConfigRoutes.getConfigRoute":
      return { requestBody: null, queryParameters: [], responses: [json("AppConfigDto")] };
    case "DeprecatedConfigRoutes.updateConfigRoute":
      return {
        requestBody: request("AppConfigUpdateDto"),
        queryParameters: [],
        responses: [empty(204), error(422, "String", "configuration persistence callback throws")],
      };
    case "DeprecatedMetadataRoutes.getProvidersRoute":
      return { requestBody: null, queryParameters: [{ name: "libraryId", type: "String", required: false }], responses: [json("List<String>")] };
    case "DeprecatedMetadataRoutes.searchSeriesRoute":
      return {
        requestBody: null,
        queryParameters: [
          { name: "name", type: "String", required: true },
          { name: "seriesId", type: "String", required: false },
          { name: "libraryId", type: "String", required: false },
        ],
        responses: [json("Collection<SeriesSearchResult>"), error(400, null, "missing name query parameter")],
      };
    case "DeprecatedMetadataRoutes.identifySeriesRoute":
      return { requestBody: request("IdentifySeriesRequest"), queryParameters: [], responses: [empty(204)] };
    case "DeprecatedMetadataRoutes.matchSeriesRoute":
      return { requestBody: null, queryParameters: [], responses: [empty(204)] };
    case "DeprecatedMetadataRoutes.matchLibraryRoute":
      return { requestBody: null, queryParameters: [], responses: [empty(202)] };
    case "DeprecatedMetadataRoutes.resetSeriesRoute":
    case "DeprecatedMetadataRoutes.resetLibraryRoute":
      return {
        requestBody: null,
        queryParameters: [{ name: "removeComicInfo", type: "Boolean", required: false, default: false }],
        responses: [empty(204)],
      };
    default:
      throw new ContractError(`Missing Komf app HTTP contract details for ${key}`);
  }
}

function buildKomfHttpApi({
  appRoutes,
  parsedSources,
  apiTypes,
  clientOperations,
  clientSourceRecords,
  komeliaSourceRecords,
  komeliaVersion,
  komeliaRef,
  komfClientVersion,
  komfClientRef,
  komeliaCatalogRecord,
}) {
  const sourceByPath = new Map(parsedSources.map((parsed) => [parsed.entry.path, parsed]));
  const serverModule = parsedSources.find((parsed) => parsed.entry.path.endsWith("/ServerModule.kt"));
  const clientFactory = sourceByPath.get(`${KOMF_CLIENT_SOURCE_PREFIX}KomfClientFactory.kt`);
  if (!serverModule || !clientFactory) throw new ContractError("Komf HTTP inventory requires ServerModule and KomfClientFactory sources");
  for (const route of ['route("/api")', 'route("/komga")', 'route("/kavita")']) {
    if (!serverModule.text.includes(route)) throw new ContractError(`ServerModule.kt no longer contains ${route}`);
  }
  for (const [className, expectedPath] of [
    ["JobRoutes", "/jobs"],
    ["MediaServerRoutes", "/media-server"],
    ["MetadataRoutes", "/metadata"],
    ["NotificationRoutes", "/notifications/discord"],
    ["NotificationRoutes", "/notifications/apprise"],
  ]) {
    if (!appRoutes.some((route) => route.verb === "ROUTE" && route.sourcePath.endsWith(`/${className}.kt`) && route.path === expectedPath)) {
      throw new ContractError(`Komf app route prefix ${expectedPath} for ${className} was not extracted`);
    }
  }

  const routes = appRoutes
    .filter((route) => ["GET", "POST", "PATCH", "PUT", "DELETE", "SSE"].includes(route.verb))
    .flatMap((route) => {
      const className = route.sourcePath.split("/").at(-1).replace(/\.kt$/, "");
      const localPrefix = routeLocalPrefix(className, route);
      const contract = appRouteContract(className, route.function);
      return routeMounts(className).map((mount) => {
        const path = pathJoin(mount.prefix, localPrefix, route.path);
        const pathParameters = [...path.matchAll(/\{([^}]+)\}/g)].map((match) => ({
          name: match[1],
          type: match[1] === "jobId" ? "UUID" : "String",
          required: true,
        }));
        const verb = route.verb === "SSE" ? "GET" : route.verb;
        const matchingClients = clientOperations.filter((operation) =>
          operation.route.method === verb && pathMatches(operation.route.path, path),
        );
        const usedClients = matchingClients.filter((operation) => operation.callSites.length > 0);
        return {
          verb,
          path,
          function: route.function,
          line: route.line,
          sourcePath: route.sourcePath,
          mediaServer: mount.mediaServer,
          deprecated: className.startsWith("Deprecated"),
          transport: route.verb === "SSE" ? "sse" : "http",
          requestBody: contract.requestBody,
          pathParameters,
          queryParameters: contract.queryParameters,
          responses: contract.responses,
          komeliaUsed: usedClients.length > 0,
          clientOperations: usedClients.map((operation) => `${operation.clientClass}.${operation.function}`).sort(),
        };
      });
    })
    .sort((left, right) => compareText(`${left.path}:${left.verb}:${left.sourcePath}:${left.line}`, `${right.path}:${right.verb}:${right.sourcePath}:${right.line}`));
  const clientOperationsWithMatches = clientOperations.map((operation) => ({
    ...operation,
    matchedRoutes: routes.filter((route) => route.verb === operation.route.method && pathMatches(operation.route.path, route.path))
      .map((route) => `${route.verb} ${route.path}`)
      .sort(),
  }));
  const eventSource = parsedSources.find((parsed) => parsed.entry.path.endsWith("/KomfMetadataJobEvents.kt"));
  const events = eventSource
    ? [...eventSource.text.matchAll(/^const val \w+ = "([^"]+)"/gm)].map((match) => match[1])
    : [];
  if (events.length === 0) throw new ContractError("KomfMetadataJobEvents.kt contains no named SSE event constants");
  const clientFactoryText = clientFactory.text;
  const baseUrl = clientFactoryText.match(/baseUrl:\s*\(\)\s*->\s*String\s*=\s*\{\s*"([^"]+)"\s*\}/)?.[1] ?? null;
  const authPluginInstalled = /\binstall\s*\(\s*Authentication\b/.test(serverModule.text);
  const authorizationConfigured = /\bAuthorization\b|\bBearer\b/.test(clientFactoryText);
  const globalErrors = [
    { exception: "IllegalArgumentException", status: 400, bodyType: "KomfErrorResponse" },
    { exception: "IllegalStateException", status: 500, bodyType: "KomfErrorResponse" },
  ];
  const omitted = routes.filter((route) => !route.komeliaUsed).map((route) => `${route.verb} ${route.path}`);
  const unmatched = clientOperationsWithMatches.filter((operation) => operation.matchedRoutes.length === 0);

  return {
    sourceCommit: parsedSources.find((parsed) => parsed.entry.path.startsWith("komf-app/"))?.ref ?? null,
    authentication: {
      serverAuthenticationPluginInstalled: authPluginInstalled,
      serverRequiresAuthentication: authPluginInstalled,
      clientDefaultAuthorizationHeaderConfigured: authorizationConfigured,
      clientDefaultBaseUrl: baseUrl,
      clientDefaultCookieStorage: clientFactoryText.includes("AcceptAllCookiesStorage()") ? "AcceptAllCookiesStorage" : null,
      clientSupportsInjectedHttpClient: clientFactoryText.includes("fun ktor(ktor: HttpClient)"),
      evidence: [
        { repository: KOMF_REPO, path: serverModule.entry.path },
        { repository: KOMF_CLIENT_REPO, path: clientFactory.entry.path },
      ],
    },
    globalErrorResponses: globalErrors,
    routes,
    routeCoverage: {
      total: routes.length,
      usedByKomelia: routes.filter((route) => route.komeliaUsed).length,
      omittedFromKomelia: omitted.length,
    },
    omittedFromKomelia: omitted,
    clientOperations: clientOperationsWithMatches,
    unmatchedClientOperations: unmatched.map((operation) => ({
      function: operation.function,
      clientClass: operation.clientClass,
      method: operation.route.method,
      path: operation.route.path,
      calledByKomelia: operation.callSites.length > 0,
      sourcePath: operation.sourcePath,
    })),
    dtoTypes: apiTypes,
    sse: {
      endpoint: "GET /api/jobs/{jobId}/events",
      contentType: "text/event-stream",
      eventNames: events,
      eventPayloads: {
        ProviderSeriesEvent: "KomfMetadataJobEvent.ProviderSeriesEvent",
        ProviderBookEvent: "KomfMetadataJobEvent.ProviderBookEvent",
        ProviderCompletedEvent: "KomfMetadataJobEvent.ProviderCompletedEvent",
        ProviderErrorEvent: "KomfMetadataJobEvent.ProviderErrorEvent",
        PostProcessingStartEvent: "KomfMetadataJobEvent.PostProcessingStartEvent",
        ProcessingErrorEvent: "KomfMetadataJobEvent.ProcessingErrorEvent",
        EventStreamNotFoundEvent: "empty SSE data; client maps to KomfMetadataJobEvent.NotFound",
      },
      completion: "CompletionEvent is filtered before emission; stream closes without a completion event",
    },
    jsonLines: routes.filter((route) => route.responses.some((response) => response.contentType === "application/jsonl"))
      .map((route) => `${route.verb} ${route.path}`),
    consumer: {
      repository: KOMELIA_REPO,
      commit: komeliaRef,
      version: komeliaVersion,
      komfClientRepository: KOMF_CLIENT_REPO,
      komfClientCommit: komfClientRef,
      komfClientVersion,
      komfClientSourceHashes: clientSourceRecords,
      komeliaSourceHashes: komeliaSourceRecords,
      gradleCatalog: komeliaCatalogRecord,
    },
  };
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
  const komeliaEntry = findUpstream(upstreams, KOMELIA_REPO);
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
    const latestContract = await extractContract({ komfEntry, komeliaEntry, ref, expectedVersion: null });
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
    komeliaEntry,
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
