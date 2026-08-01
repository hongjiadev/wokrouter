import { readFile, readdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as tsAst from "typescript/unstable/ast";
import { API as TypeScriptApi } from "typescript/unstable/sync";

const expectedTopLevelNamespaces = [
  "app",
  "common",
  "core",
  "errors",
  "management",
  "operation",
];
const expectedManagementNamespaces = [
  "diagnostics",
  "providers",
  "sessions",
  "usage",
];
const htmlElementMarkup =
  /<\/?[A-Za-z][A-Za-z0-9-]*(?:\s+[^<>]*?)?\s*\/?>/u;
const htmlCommentMarkup = /<!--[\s\S]*?-->/u;
const htmlDoctypeMarkup = /<!doctype(?:\s+[^<>]*?)?>/iu;
const htmlProcessingMarkup = /<\?[A-Za-z][\s\S]*?\?>/u;
const interpolationPlaceholder = /{{\s*([A-Za-z0-9_.-]+)\s*}}/g;
const visibleStringAttributes = new Set([
  "alt",
  "aria-description",
  "aria-label",
  "aria-valuetext",
  "placeholder",
  "title",
]);
const visibleTechnicalLiterals = new Set(["WokCore", "WokRouter"]);
const translatedCharacter = /[\p{Script=Han}\p{Script=Latin}]/u;

function containsHtmlMarkup(value) {
  return (
    htmlElementMarkup.test(value) ||
    htmlCommentMarkup.test(value) ||
    htmlDoctypeMarkup.test(value) ||
    htmlProcessingMarkup.test(value)
  );
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function requirePlainNamespace(value, locale, path) {
  if (!isPlainObject(value)) {
    throw new Error(
      `Catalog "${locale}" namespace "${path}" must be a plain object.`,
    );
  }
}

function requireExactNamespaces(catalog, locale, path, expected) {
  requirePlainNamespace(catalog, locale, path);
  const actual = Object.keys(catalog).sort();
  if (
    actual.length !== expected.length ||
    actual.some((key, index) => key !== expected[index])
  ) {
    throw new Error(
      `Catalog "${locale}" namespace "${path}" must contain exactly: ${expected.join(
        ", ",
      )}.`,
    );
  }
}

function flattenCatalog(catalog, locale, prefix = "", leaves = new Map()) {
  for (const key of Object.keys(catalog).sort()) {
    const namespace = prefix || "<root>";
    if (key === "") {
      throw new Error(
        `Catalog "${locale}" namespace "${namespace}" contains an empty key segment.`,
      );
    }
    if (key.includes(".")) {
      throw new Error(
        `Catalog "${locale}" namespace "${namespace}" contains dotted key segment "${key}".`,
      );
    }
    const value = catalog[key];
    const path = prefix ? `${prefix}.${key}` : key;
    if (isPlainObject(value)) {
      flattenCatalog(value, locale, path, leaves);
      continue;
    }
    if (typeof value !== "string" || value.trim() === "") {
      throw new Error(
        `Catalog "${locale}" key "${path}" must be a non-empty string.`,
      );
    }
    if (containsHtmlMarkup(value)) {
      throw new Error(
        `Catalog "${locale}" key "${path}" must not contain HTML markup.`,
      );
    }
    leaves.set(path, value);
  }
  return leaves;
}

function placeholders(value) {
  return [
    ...new Set(
      [...value.matchAll(interpolationPlaceholder)].map((match) => match[1]),
    ),
  ].sort();
}

function sameStrings(left, right) {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

export function validateCatalogs(english, simplifiedChinese) {
  requireExactNamespaces(
    english,
    "en",
    "<root>",
    expectedTopLevelNamespaces,
  );
  requireExactNamespaces(
    simplifiedChinese,
    "zh-CN",
    "<root>",
    expectedTopLevelNamespaces,
  );
  for (const namespace of expectedTopLevelNamespaces) {
    requirePlainNamespace(english[namespace], "en", namespace);
    requirePlainNamespace(simplifiedChinese[namespace], "zh-CN", namespace);
  }
  requireExactNamespaces(
    english.management,
    "en",
    "management",
    expectedManagementNamespaces,
  );
  requireExactNamespaces(
    simplifiedChinese.management,
    "zh-CN",
    "management",
    expectedManagementNamespaces,
  );
  for (const namespace of expectedManagementNamespaces) {
    const path = `management.${namespace}`;
    requirePlainNamespace(english.management[namespace], "en", path);
    requirePlainNamespace(
      simplifiedChinese.management[namespace],
      "zh-CN",
      path,
    );
  }

  const catalogs = [
    ["en", flattenCatalog(english, "en")],
    ["zh-CN", flattenCatalog(simplifiedChinese, "zh-CN")],
  ];
  const [englishLocale, englishLeaves] = catalogs[0];
  const [chineseLocale, chineseLeaves] = catalogs[1];
  const keys = [...new Set([...englishLeaves.keys(), ...chineseLeaves.keys()])]
    .sort();

  for (const key of keys) {
    if (!englishLeaves.has(key)) {
      throw new Error(`Catalog "${englishLocale}" is missing key "${key}".`);
    }
    if (!chineseLeaves.has(key)) {
      throw new Error(`Catalog "${chineseLocale}" is missing key "${key}".`);
    }
    const englishPlaceholders = placeholders(englishLeaves.get(key));
    const chinesePlaceholders = placeholders(chineseLeaves.get(key));
    if (!sameStrings(englishPlaceholders, chinesePlaceholders)) {
      throw new Error(
        `Catalog placeholder mismatch at "${key}": ` +
          `en has [${englishPlaceholders.join(", ")}], ` +
          `zh-CN has [${chinesePlaceholders.join(", ")}].`,
      );
    }
  }

  return keys.length;
}

function normalizedVisibleText(value) {
  return value.replace(/\s+/gu, " ").trim();
}

function jsxStringAttribute(openingElement, sourceFile, name) {
  for (const attribute of openingElement.attributes.properties) {
    if (
      tsAst.isJsxAttribute(attribute) &&
      attribute.name.getText(sourceFile) === name &&
      attribute.initializer !== undefined &&
      tsAst.isStringLiteral(attribute.initializer)
    ) {
      return attribute.initializer.text;
    }
  }
  return undefined;
}

function isVisibleTechnicalLiteral(sourceFile, node, text) {
  if (visibleTechnicalLiterals.has(text)) {
    return true;
  }
  if (
    text !== "W" ||
    !/(?:^|[\\/])src[\\/]App\.tsx$/u.test(sourceFile.fileName) ||
    !tsAst.isJsxElement(node.parent)
  ) {
    return false;
  }
  const openingElement = node.parent.openingElement;
  return (
    openingElement.tagName.getText(sourceFile) === "span" &&
    jsxStringAttribute(openingElement, sourceFile, "className") ===
      "brand-mark" &&
    jsxStringAttribute(openingElement, sourceFile, "aria-hidden") === "true"
  );
}

function assertTranslatedLiteral(sourceFile, node, value, context) {
  const text = normalizedVisibleText(value);
  if (
    text === "" ||
    !translatedCharacter.test(text) ||
    isVisibleTechnicalLiteral(sourceFile, node, text)
  ) {
    return;
  }
  const position = sourceFile.getLineAndCharacterOfPosition(node.getStart());
  throw new Error(
    `Untranslated user-facing text ${JSON.stringify(text)} in ` +
      `${sourceFile.fileName}:${position.line + 1}:${position.character + 1} ` +
      `(${context}).`,
  );
}

function unwrapAuditedExpression(expression) {
  let current = expression;
  while (
    tsAst.isParenthesizedExpression(current) ||
    tsAst.isAsExpression(current) ||
    tsAst.isSatisfiesExpression(current) ||
    tsAst.isTypeAssertion(current) ||
    tsAst.isNonNullExpression(current)
  ) {
    current = current.expression;
  }
  return current;
}

function propertyName(sourceFile, name) {
  if (
    tsAst.isIdentifier(name) ||
    tsAst.isStringLiteral(name) ||
    tsAst.isNoSubstitutionTemplateLiteral(name) ||
    tsAst.isNumericLiteral(name)
  ) {
    return name.text;
  }
  if (
    tsAst.isComputedPropertyName(name) &&
    (tsAst.isStringLiteral(name.expression) ||
      tsAst.isNoSubstitutionTemplateLiteral(name.expression))
  ) {
    return name.expression.text;
  }
  return name.getText(sourceFile);
}

function visibleObjectPropertyInitializer(
  sourceFile,
  checker,
  expression,
  name,
  boundExpressions,
) {
  let objectExpression = unwrapAuditedExpression(expression);
  const binding = visibleBindingInitializer(
    sourceFile,
    checker,
    objectExpression,
    boundExpressions,
  );
  if (binding !== undefined) {
    objectExpression = unwrapAuditedExpression(binding.initializer);
  }
  if (!tsAst.isObjectLiteralExpression(objectExpression)) {
    return undefined;
  }
  for (const property of objectExpression.properties) {
    if (
      (tsAst.isPropertyAssignment(property) ||
        tsAst.isShorthandPropertyAssignment(property)) &&
      propertyName(sourceFile, property.name) === name
    ) {
      return tsAst.isPropertyAssignment(property)
        ? property.initializer
        : property.name;
    }
  }
  return undefined;
}

function visibleBindingInitializer(
  sourceFile,
  checker,
  expression,
  boundExpressions,
) {
  const reference = tsAst.isIdentifier(expression)
    ? expression
    : tsAst.isPropertyAccessExpression(expression)
      ? expression.name
      : undefined;
  if (reference === undefined) {
    return undefined;
  }
  const symbol = checker.getSymbolAtLocation(reference);
  if (symbol !== undefined && boundExpressions.has(symbol.id)) {
    return {
      initializer: boundExpressions.get(symbol.id),
      symbolId: symbol.id,
    };
  }
  const declaration = symbol?.valueDeclaration?.resolve();
  if (
    declaration === undefined ||
    declaration.getSourceFile().fileName !== sourceFile.fileName
  ) {
    return undefined;
  }
  if (
    (tsAst.isVariableDeclaration(declaration) ||
      tsAst.isBindingElement(declaration) ||
      tsAst.isParameterDeclaration(declaration) ||
      tsAst.isPropertyAssignment(declaration) ||
      tsAst.isPropertyDeclaration(declaration)) &&
    declaration.initializer !== undefined
  ) {
    return { initializer: declaration.initializer, symbolId: symbol.id };
  }
  if (!tsAst.isBindingElement(declaration)) {
    return undefined;
  }
  const pattern = declaration.parent;
  const owner = pattern.parent;
  if (
    owner.initializer === undefined ||
    !(
      tsAst.isVariableDeclaration(owner) ||
      tsAst.isBindingElement(owner) ||
      tsAst.isParameterDeclaration(owner)
    )
  ) {
    return undefined;
  }
  if (tsAst.isObjectBindingPattern(pattern)) {
    const name = propertyName(
      sourceFile,
      declaration.propertyName ?? declaration.name,
    );
    const initializer = visibleObjectPropertyInitializer(
      sourceFile,
      checker,
      owner.initializer,
      name,
      boundExpressions,
    );
    return initializer === undefined
      ? undefined
      : { initializer, symbolId: symbol.id };
  }
  if (tsAst.isArrayBindingPattern(pattern)) {
    const index = pattern.elements.indexOf(declaration);
    const initializer = unwrapAuditedExpression(owner.initializer);
    if (
      index >= 0 &&
      tsAst.isArrayLiteralExpression(initializer) &&
      index < initializer.elements.length &&
      !tsAst.isOmittedExpression(initializer.elements[index])
    ) {
      return {
        initializer: initializer.elements[index],
        symbolId: symbol.id,
      };
    }
  }
  return undefined;
}

function staticLiteralValue(
  sourceFile,
  checker,
  expression,
  boundExpressions,
  visitedSymbols = new Set(),
) {
  const candidate = unwrapAuditedExpression(expression);
  if (
    tsAst.isStringLiteral(candidate) ||
    tsAst.isNoSubstitutionTemplateLiteral(candidate) ||
    tsAst.isNumericLiteral(candidate)
  ) {
    return candidate.text;
  }
  const binding = visibleBindingInitializer(
    sourceFile,
    checker,
    candidate,
    boundExpressions,
  );
  if (
    binding === undefined ||
    visitedSymbols.has(binding.symbolId)
  ) {
    return undefined;
  }
  visitedSymbols.add(binding.symbolId);
  return staticLiteralValue(
    sourceFile,
    checker,
    binding.initializer,
    boundExpressions,
    visitedSymbols,
  );
}

function visibleElementInitializer(
  sourceFile,
  checker,
  expression,
  boundExpressions,
) {
  if (!tsAst.isElementAccessExpression(expression)) {
    return undefined;
  }
  const key = staticLiteralValue(
    sourceFile,
    checker,
    expression.argumentExpression,
    boundExpressions,
  );
  if (key === undefined) {
    return undefined;
  }
  let container = unwrapAuditedExpression(expression.expression);
  const containerBinding = visibleBindingInitializer(
    sourceFile,
    checker,
    container,
    boundExpressions,
  );
  if (containerBinding !== undefined) {
    container = unwrapAuditedExpression(containerBinding.initializer);
  }
  let initializer;
  if (tsAst.isArrayLiteralExpression(container) && /^\d+$/u.test(key)) {
    const index = Number(key);
    if (
      index < container.elements.length &&
      !tsAst.isOmittedExpression(container.elements[index])
    ) {
      initializer = container.elements[index];
    }
  } else {
    initializer = visibleObjectPropertyInitializer(
      sourceFile,
      checker,
      container,
      key,
      boundExpressions,
    );
  }
  return initializer === undefined
    ? undefined
    : {
        initializer,
        symbolId: `element:${expression.getStart(sourceFile)}`,
      };
}

function bindCallArgument(
  sourceFile,
  checker,
  pattern,
  argument,
  argumentBindings,
) {
  if (tsAst.isIdentifier(pattern)) {
    const symbol = checker.getSymbolAtLocation(pattern);
    if (symbol !== undefined) {
      argumentBindings.set(symbol.id, argument);
    }
    return;
  }
  if (tsAst.isObjectBindingPattern(pattern)) {
    for (const element of pattern.elements) {
      const name = propertyName(
        sourceFile,
        element.propertyName ?? element.name,
      );
      const property = visibleObjectPropertyInitializer(
        sourceFile,
        checker,
        argument,
        name,
        argumentBindings,
      );
      if (property !== undefined) {
        bindCallArgument(
          sourceFile,
          checker,
          element.name,
          property,
          argumentBindings,
        );
      }
    }
    return;
  }
  if (!tsAst.isArrayBindingPattern(pattern)) {
    return;
  }
  let array = unwrapAuditedExpression(argument);
  const binding = visibleBindingInitializer(
    sourceFile,
    checker,
    array,
    argumentBindings,
  );
  if (binding !== undefined) {
    array = unwrapAuditedExpression(binding.initializer);
  }
  if (!tsAst.isArrayLiteralExpression(array)) {
    return;
  }
  for (let index = 0; index < pattern.elements.length; index += 1) {
    const element = pattern.elements[index];
    const value = array.elements[index];
    if (
      tsAst.isOmittedExpression(element) ||
      value === undefined ||
      tsAst.isOmittedExpression(value)
    ) {
      continue;
    }
    bindCallArgument(
      sourceFile,
      checker,
      element.name,
      value,
      argumentBindings,
    );
  }
}

function visibleLocalCallResults(
  sourceFile,
  checker,
  expression,
  boundExpressions,
) {
  if (!tsAst.isCallExpression(expression)) {
    return undefined;
  }
  const callee = unwrapAuditedExpression(expression.expression);
  let functionLike;
  let symbolId;
  if (
    tsAst.isArrowFunction(callee) ||
    tsAst.isFunctionExpression(callee)
  ) {
    functionLike = callee;
    symbolId = `call:${callee.getStart(sourceFile)}`;
  }
  const reference = tsAst.isIdentifier(callee)
    ? callee
    : tsAst.isPropertyAccessExpression(callee)
      ? callee.name
      : undefined;
  if (functionLike === undefined && reference !== undefined) {
    const symbol = checker.getSymbolAtLocation(reference);
    const declaration = symbol?.valueDeclaration?.resolve();
    if (
      tsAst.isIdentifier(reference) &&
      reference.text === "String" &&
      declaration?.getSourceFile().fileName !== sourceFile.fileName
    ) {
      return {
        argumentBindings: boundExpressions,
        expressions: expression.arguments.slice(0, 1),
        symbolId: "global:String",
      };
    }
    if (
      declaration === undefined ||
      declaration.getSourceFile().fileName !== sourceFile.fileName
    ) {
      return undefined;
    }
    functionLike = tsAst.isFunctionLikeDeclaration(declaration)
      ? declaration
      : "initializer" in declaration &&
          declaration.initializer !== undefined &&
          tsAst.isFunctionLikeDeclaration(declaration.initializer)
        ? declaration.initializer
        : undefined;
    symbolId = symbol.id;
  }
  if (functionLike?.body === undefined) {
    return undefined;
  }
  const argumentBindings = new Map(boundExpressions);
  for (let index = 0; index < functionLike.parameters.length; index += 1) {
    const parameter = functionLike.parameters[index];
    const argument = expression.arguments[index];
    if (argument === undefined) {
      continue;
    }
    bindCallArgument(
      sourceFile,
      checker,
      parameter.name,
      argument,
      argumentBindings,
    );
  }
  if (!tsAst.isBlock(functionLike.body)) {
    return {
      argumentBindings,
      expressions: [functionLike.body],
      symbolId,
    };
  }

  const expressions = [];
  function visit(node) {
    if (node !== functionLike.body && tsAst.isFunctionLikeDeclaration(node)) {
      return;
    }
    if (tsAst.isReturnStatement(node) && node.expression !== undefined) {
      expressions.push(node.expression);
      return;
    }
    node.forEachChild(visit);
  }
  visit(functionLike.body);
  return { argumentBindings, expressions, symbolId };
}

function auditVisibleExpression(
  sourceFile,
  checker,
  expression,
  context,
  visitedSymbols = new Set(),
  boundExpressions = new Map(),
) {
  if (
    tsAst.isStringLiteral(expression) ||
    tsAst.isNoSubstitutionTemplateLiteral(expression)
  ) {
    assertTranslatedLiteral(sourceFile, expression, expression.text, context);
    return;
  }
  if (tsAst.isTemplateExpression(expression)) {
    assertTranslatedLiteral(
      sourceFile,
      expression.head,
      expression.head.text,
      context,
    );
    for (const span of expression.templateSpans) {
      auditVisibleExpression(
        sourceFile,
        checker,
        span.expression,
        context,
        visitedSymbols,
        boundExpressions,
      );
      assertTranslatedLiteral(
        sourceFile,
        span.literal,
        span.literal.text,
        context,
      );
    }
    return;
  }
  if (tsAst.isParenthesizedExpression(expression)) {
    auditVisibleExpression(
      sourceFile,
      checker,
      expression.expression,
      context,
      visitedSymbols,
      boundExpressions,
    );
    return;
  }
  if (
    tsAst.isAsExpression(expression) ||
    tsAst.isSatisfiesExpression(expression) ||
    tsAst.isTypeAssertion(expression) ||
    tsAst.isNonNullExpression(expression)
  ) {
    auditVisibleExpression(
      sourceFile,
      checker,
      expression.expression,
      context,
      visitedSymbols,
      boundExpressions,
    );
    return;
  }
  if (tsAst.isConditionalExpression(expression)) {
    auditVisibleExpression(
      sourceFile,
      checker,
      expression.whenTrue,
      context,
      visitedSymbols,
      boundExpressions,
    );
    auditVisibleExpression(
      sourceFile,
      checker,
      expression.whenFalse,
      context,
      visitedSymbols,
      boundExpressions,
    );
    return;
  }
  if (tsAst.isBinaryExpression(expression)) {
    const operator = expression.operatorToken.kind;
    if (
      operator === tsAst.SyntaxKind.PlusToken ||
      operator === tsAst.SyntaxKind.BarBarToken ||
      operator === tsAst.SyntaxKind.QuestionQuestionToken
    ) {
      auditVisibleExpression(
        sourceFile,
        checker,
        expression.left,
        context,
        visitedSymbols,
        boundExpressions,
      );
      auditVisibleExpression(
        sourceFile,
        checker,
        expression.right,
        context,
        visitedSymbols,
        boundExpressions,
      );
    } else if (operator === tsAst.SyntaxKind.AmpersandAmpersandToken) {
      auditVisibleExpression(
        sourceFile,
        checker,
        expression.right,
        context,
        visitedSymbols,
        boundExpressions,
      );
    }
    return;
  }
  const localCall = visibleLocalCallResults(
    sourceFile,
    checker,
    expression,
    boundExpressions,
  );
  if (
    localCall !== undefined &&
    !visitedSymbols.has(localCall.symbolId)
  ) {
    visitedSymbols.add(localCall.symbolId);
    for (const result of localCall.expressions) {
      auditVisibleExpression(
        sourceFile,
        checker,
        result,
        context,
        visitedSymbols,
        localCall.argumentBindings,
      );
    }
    visitedSymbols.delete(localCall.symbolId);
    return;
  }
  if (tsAst.isCallExpression(expression)) {
    const callee = unwrapAuditedExpression(expression.expression);
    const translationCall =
      (tsAst.isIdentifier(callee) && callee.text === "t") ||
      (tsAst.isPropertyAccessExpression(callee) &&
        callee.name.text === "t");
    if (!translationCall) {
      if (
        tsAst.isPropertyAccessExpression(callee) ||
        tsAst.isElementAccessExpression(callee)
      ) {
        auditVisibleExpression(
          sourceFile,
          checker,
          callee.expression,
          context,
          visitedSymbols,
          boundExpressions,
        );
      }
      for (const argument of expression.arguments) {
        auditVisibleExpression(
          sourceFile,
          checker,
          argument,
          context,
          visitedSymbols,
          boundExpressions,
        );
      }
    }
    return;
  }
  const binding =
    visibleElementInitializer(
      sourceFile,
      checker,
      expression,
      boundExpressions,
    ) ??
    visibleBindingInitializer(
      sourceFile,
      checker,
      expression,
      boundExpressions,
    );
  if (
    binding !== undefined &&
    !visitedSymbols.has(binding.symbolId)
  ) {
    visitedSymbols.add(binding.symbolId);
    auditVisibleExpression(
      sourceFile,
      checker,
      binding.initializer,
      context,
      visitedSymbols,
      boundExpressions,
    );
    visitedSymbols.delete(binding.symbolId);
    return;
  }
  if (tsAst.isArrayLiteralExpression(expression)) {
    for (const element of expression.elements) {
      if (!tsAst.isSpreadElement(element)) {
        auditVisibleExpression(
          sourceFile,
          checker,
          element,
          context,
          visitedSymbols,
          boundExpressions,
        );
      }
    }
  }
}

function validateProductionTsxAst(sourceFile, checker) {
  function visit(node) {
    if (tsAst.isJsxText(node)) {
      assertTranslatedLiteral(sourceFile, node, node.text, "JSX child");
    } else if (
      tsAst.isJsxExpression(node) &&
      !tsAst.isJsxAttribute(node.parent) &&
      node.expression !== undefined
    ) {
      auditVisibleExpression(
        sourceFile,
        checker,
        node.expression,
        "JSX child",
      );
    } else if (tsAst.isJsxAttribute(node)) {
      const attributeName = node.name.getText(sourceFile);
      if (visibleStringAttributes.has(attributeName)) {
        if (
          node.initializer &&
          tsAst.isStringLiteral(node.initializer)
        ) {
          assertTranslatedLiteral(
            sourceFile,
            node.initializer,
            node.initializer.text,
            `${attributeName} attribute`,
          );
        } else if (
          node.initializer &&
          tsAst.isJsxExpression(node.initializer) &&
          node.initializer.expression !== undefined
        ) {
          auditVisibleExpression(
            sourceFile,
            checker,
            node.initializer.expression,
            `${attributeName} attribute`,
          );
        }
      }
    }
    node.forEachChild(visit);
  }

  visit(sourceFile);
  return 0;
}

async function productionTsxPaths(directory) {
  const paths = [];
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort(
    (left, right) => left.name.localeCompare(right.name),
  )) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "__tests__") {
        paths.push(...(await productionTsxPaths(path)));
      }
      continue;
    }
    if (
      entry.isFile() &&
      entry.name.endsWith(".tsx") &&
      !/\.(?:test|spec)\.tsx$/u.test(entry.name)
    ) {
      paths.push(path);
    }
  }
  return paths;
}

export async function auditProductionTsx(desktopDirectory) {
  const sourceDirectory = resolve(desktopDirectory, "src");
  const paths = await productionTsxPaths(sourceDirectory);
  const configPath = resolve(desktopDirectory, "tsconfig.json");
  const api = new TypeScriptApi();
  let snapshot;
  try {
    snapshot = api.updateSnapshot({
      openProjects: [configPath],
      openFiles: paths,
    });
    for (const path of paths) {
      const project = snapshot.getDefaultProjectForFile(path);
      const sourceFile = project?.program.getSourceFile(path);
      if (sourceFile === undefined) {
        throw new Error(
          `TypeScript did not load production TSX ${JSON.stringify(path)}.`,
        );
      }
      const diagnostic = project.program.getSyntacticDiagnostics(path)[0];
      if (diagnostic !== undefined) {
        throw new Error(
          `Production TSX ${JSON.stringify(path)} could not be parsed: ` +
            String(diagnostic.messageText),
        );
      }
      validateProductionTsxAst(sourceFile, project.checker);
    }
  } finally {
    snapshot?.dispose();
    api.close();
  }
  return paths.length;
}

async function readCatalog(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function resolveCatalogPaths(desktopDirectory, arguments_) {
  const paths = arguments_[0] === "--" ? arguments_.slice(1) : arguments_;
  if (paths.length === 0) {
    const localeDirectory = resolve(desktopDirectory, "src", "i18n", "locales");
    return [
      resolve(localeDirectory, "en.json"),
      resolve(localeDirectory, "zh-CN.json"),
    ];
  }
  if (paths.length !== 2) {
    throw new Error(
      "Usage: check-i18n-catalogs.mjs [<en.json> <zh-CN.json>]",
    );
  }
  return paths.map((path) => resolve(path));
}

async function main(arguments_ = process.argv.slice(2)) {
  const desktopDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const [englishPath, chinesePath] = resolveCatalogPaths(
    desktopDirectory,
    arguments_,
  );
  const english = await readCatalog(englishPath);
  const simplifiedChinese = await readCatalog(chinesePath);
  const keyCount = validateCatalogs(english, simplifiedChinese);
  const sourceFileCount = await auditProductionTsx(desktopDirectory);
  process.stdout.write(`Translation catalogs match (${keyCount} keys).\n`);
  process.stdout.write(
    `Production TSX copy audit passed (${sourceFileCount} files).\n`,
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    process.stderr.write(
      `${error instanceof Error ? error.message : "Catalog validation failed."}\n`,
    );
    process.exitCode = 1;
  });
}
