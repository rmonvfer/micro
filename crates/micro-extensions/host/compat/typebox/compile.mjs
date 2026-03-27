// Real typebox 1.1.38 (https://www.npmjs.com/package/typebox), MIT licensed, bundled with
// `bun build --target=browser --format=esm` from its published .mjs build so it carries no
// external imports of its own — the same reason pi statically bundles it into its own
// compiled binary. An extension that imports `Type.Object(...)` from `typebox` or
// `@sinclair/typebox` to describe a tool's parameters gets the genuine library, not an
// approximation of it. Regenerate by running the same `bun build` command above against a
// newer `typebox` version's build/*/index.mjs.

var __defProp = Object.defineProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, {
      get: all[name],
      enumerable: true,
      configurable: true,
      set: (newValue) => all[name] = () => newValue
    });
};

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/arguments/arguments.mjs
var exports_arguments = {};
__export(exports_arguments, {
  Match: () => Match
});
function Match(args, match) {
  return match[args.length]?.(...args) ?? (() => {
    throw Error("Invalid Arguments");
  })();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/emit.mjs
var exports_emit = {};
__export(exports_emit, {
  Ternary: () => Ternary,
  Statements: () => Statements,
  Return: () => Return,
  ReduceOr: () => ReduceOr,
  ReduceAnd: () => ReduceAnd,
  PrefixIncrement: () => PrefixIncrement,
  Or: () => Or,
  Not: () => Not,
  New: () => New,
  MultipleOf: () => MultipleOf,
  Member: () => Member,
  Keys: () => Keys2,
  IsUndefined: () => IsUndefined2,
  IsSymbol: () => IsSymbol2,
  IsString: () => IsString2,
  IsObjectNotArray: () => IsObjectNotArray2,
  IsObject: () => IsObject2,
  IsNumber: () => IsNumber2,
  IsNull: () => IsNull2,
  IsMinLength: () => IsMinLength3,
  IsMaxLength: () => IsMaxLength3,
  IsLessThan: () => IsLessThan2,
  IsLessEqualThan: () => IsLessEqualThan2,
  IsIterator: () => IsIterator2,
  IsInteger: () => IsInteger2,
  IsGreaterThan: () => IsGreaterThan2,
  IsGreaterEqualThan: () => IsGreaterEqualThan2,
  IsFunction: () => IsFunction2,
  IsEqual: () => IsEqual2,
  IsDeepEqual: () => IsDeepEqual2,
  IsConstructor: () => IsConstructor2,
  IsBoolean: () => IsBoolean2,
  IsBigInt: () => IsBigInt2,
  IsAsyncIterator: () => IsAsyncIterator2,
  IsArray: () => IsArray2,
  If: () => If,
  HasPropertyKey: () => HasPropertyKey2,
  Every: () => Every2,
  Entries: () => Entries2,
  Constant: () => Constant,
  ConstDeclaration: () => ConstDeclaration,
  Call: () => Call,
  ArrowFunction: () => ArrowFunction,
  ArrayLiteral: () => ArrayLiteral,
  And: () => And
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/guard.mjs
var exports_guard = {};
__export(exports_guard, {
  Values: () => Values,
  TakeLeft: () => TakeLeft,
  Symbols: () => Symbols,
  Keys: () => Keys,
  IsValueLike: () => IsValueLike,
  IsUnsafePropertyKey: () => IsUnsafePropertyKey,
  IsUndefined: () => IsUndefined,
  IsSymbol: () => IsSymbol,
  IsString: () => IsString,
  IsObjectNotArray: () => IsObjectNotArray,
  IsObject: () => IsObject,
  IsNumber: () => IsNumber,
  IsNull: () => IsNull,
  IsMultipleOf: () => IsMultipleOf,
  IsMinLength: () => IsMinLength2,
  IsMaxLength: () => IsMaxLength2,
  IsLessThan: () => IsLessThan,
  IsLessEqualThan: () => IsLessEqualThan,
  IsIterator: () => IsIterator,
  IsInteger: () => IsInteger,
  IsGreaterThan: () => IsGreaterThan,
  IsGreaterEqualThan: () => IsGreaterEqualThan,
  IsFunction: () => IsFunction,
  IsEqual: () => IsEqual,
  IsDeepEqual: () => IsDeepEqual,
  IsConstructor: () => IsConstructor,
  IsClassInstance: () => IsClassInstance,
  IsBoolean: () => IsBoolean,
  IsBigInt: () => IsBigInt,
  IsAsyncIterator: () => IsAsyncIterator,
  IsArray: () => IsArray,
  HasPropertyKey: () => HasPropertyKey,
  GraphemeCount: () => GraphemeCount2,
  EveryAll: () => EveryAll,
  Every: () => Every,
  EntriesRegExp: () => EntriesRegExp,
  Entries: () => Entries
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/string.mjs
function IsBetween(value, min, max) {
  return value >= min && value <= max;
}
function IsRegionalIndicator(value) {
  return IsBetween(value, 127462, 127487);
}
function IsVariationSelector(value) {
  return IsBetween(value, 65024, 65039);
}
function IsCombiningMark(value) {
  return IsBetween(value, 768, 879) || IsBetween(value, 6832, 6911) || IsBetween(value, 7616, 7679) || IsBetween(value, 65056, 65071);
}
function CodePointLength(value) {
  return value > 65535 ? 2 : 1;
}
function ConsumeModifiers(value, index) {
  while (index < value.length) {
    const point = value.codePointAt(index);
    if (IsCombiningMark(point) || IsVariationSelector(point)) {
      index += CodePointLength(point);
    } else {
      break;
    }
  }
  return index;
}
function NextGraphemeClusterIndex(value, clusterStart) {
  const startCP = value.codePointAt(clusterStart);
  let clusterEnd = clusterStart + CodePointLength(startCP);
  clusterEnd = ConsumeModifiers(value, clusterEnd);
  while (clusterEnd < value.length - 1 && value[clusterEnd] === "‍") {
    const nextCP = value.codePointAt(clusterEnd + 1);
    clusterEnd += 1 + CodePointLength(nextCP);
    clusterEnd = ConsumeModifiers(value, clusterEnd);
  }
  if (IsRegionalIndicator(startCP) && clusterEnd < value.length && IsRegionalIndicator(value.codePointAt(clusterEnd))) {
    clusterEnd += CodePointLength(value.codePointAt(clusterEnd));
  }
  return clusterEnd;
}
function IsGraphemeCodePoint(value) {
  return IsBetween(value, 55296, 56319) || IsBetween(value, 768, 879) || value === 8205;
}
function GraphemeCount(value) {
  let count = 0;
  let index = 0;
  while (index < value.length) {
    index = NextGraphemeClusterIndex(value, index);
    count++;
  }
  return count;
}
function IsMinLength(value, minLength) {
  if (minLength === 0)
    return true;
  let count = 0;
  let index = 0;
  while (index < value.length) {
    index = NextGraphemeClusterIndex(value, index);
    count++;
    if (count >= minLength)
      return true;
  }
  return false;
}
function IsMaxLength(value, maxLength) {
  let count = 0;
  let index = 0;
  while (index < value.length) {
    index = NextGraphemeClusterIndex(value, index);
    count++;
    if (count > maxLength)
      return false;
  }
  return true;
}
function IsMinLengthFast(value, minLength) {
  if (minLength === 0)
    return true;
  let index = 0;
  while (index < value.length) {
    if (IsGraphemeCodePoint(value.charCodeAt(index))) {
      return IsMinLength(value, minLength);
    }
    index++;
    if (index >= minLength)
      return true;
  }
  return false;
}
function IsMaxLengthFast(value, maxLength) {
  let index = 0;
  while (index < value.length) {
    if (IsGraphemeCodePoint(value.charCodeAt(index))) {
      return IsMaxLength(value, maxLength);
    }
    index++;
    if (index > maxLength)
      return false;
  }
  return true;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/guard.mjs
function IsArray(value) {
  return Array.isArray(value);
}
function IsAsyncIterator(value) {
  return IsObject(value) && Symbol.asyncIterator in value;
}
function IsBigInt(value) {
  return IsEqual(typeof value, "bigint");
}
function IsBoolean(value) {
  return IsEqual(typeof value, "boolean");
}
function IsConstructor(value) {
  if (IsUndefined(value) || !IsFunction(value))
    return false;
  const result = Function.prototype.toString.call(value);
  if (/^class\s/.test(result))
    return true;
  if (/\[native code\]/.test(result))
    return true;
  return false;
}
function IsFunction(value) {
  return IsEqual(typeof value, "function");
}
function IsInteger(value) {
  return Number.isInteger(value);
}
function IsIterator(value) {
  return IsObject(value) && Symbol.iterator in value;
}
function IsNull(value) {
  return IsEqual(value, null);
}
function IsNumber(value) {
  return Number.isFinite(value);
}
function IsObjectNotArray(value) {
  return IsObject(value) && !IsArray(value);
}
function IsObject(value) {
  return IsEqual(typeof value, "object") && !IsNull(value);
}
function IsString(value) {
  return IsEqual(typeof value, "string");
}
function IsSymbol(value) {
  return IsEqual(typeof value, "symbol");
}
function IsUndefined(value) {
  return IsEqual(value, undefined);
}
function IsEqual(left, right) {
  return left === right;
}
function IsGreaterThan(left, right) {
  return left > right;
}
function IsLessThan(left, right) {
  return left < right;
}
function IsLessEqualThan(left, right) {
  return left <= right;
}
function IsGreaterEqualThan(left, right) {
  return left >= right;
}
function IsMultipleOf(dividend, divisor) {
  if (IsBigInt(dividend) || IsBigInt(divisor)) {
    return BigInt(dividend) % BigInt(divisor) === 0n;
  }
  const tolerance = 0.0000000001;
  if (!IsNumber(dividend))
    return true;
  if (IsInteger(dividend) && 1 / divisor % 1 === 0)
    return true;
  const mod = dividend % divisor;
  return Math.min(Math.abs(mod), Math.abs(mod - divisor)) < tolerance;
}
function IsClassInstance(value) {
  if (!IsObject(value))
    return false;
  const proto = globalThis.Object.getPrototypeOf(value);
  if (IsNull(proto))
    return false;
  return IsEqual(typeof proto.constructor, "function") && !(IsEqual(proto.constructor, globalThis.Object) || IsEqual(proto.constructor.name, "Object"));
}
function IsValueLike(value) {
  return IsBigInt(value) || IsBoolean(value) || IsNull(value) || IsNumber(value) || IsString(value) || IsUndefined(value);
}
function GraphemeCount2(value) {
  return GraphemeCount(value);
}
function IsMaxLength2(value, length) {
  return IsMaxLengthFast(value, length);
}
function IsMinLength2(value, length) {
  return IsMinLengthFast(value, length);
}
function Every(value, offset, callback) {
  for (let index = offset;index < value.length; index++) {
    if (!callback(value[index], index))
      return false;
  }
  return true;
}
function EveryAll(value, offset, callback) {
  let result = true;
  for (let index = offset;index < value.length; index++) {
    if (!callback(value[index], index))
      result = false;
  }
  return result;
}
function TakeLeft(array, true_, false_) {
  return IsEqual(array.length, 0) ? false_() : true_(array[0], array.slice(1));
}
function IsUnsafePropertyKey(key) {
  return IsEqual(key, "__proto__") || IsEqual(key, "constructor") || IsEqual(key, "prototype");
}
function HasPropertyKey(value, key) {
  return IsUnsafePropertyKey(key) ? Object.prototype.hasOwnProperty.call(value, key) : (key in value);
}
function EntriesRegExp(value) {
  return Keys(value).map((key) => [new RegExp(`^${key}$`), value[key]]);
}
function Entries(value) {
  return Object.entries(value);
}
function Keys(value) {
  return Object.getOwnPropertyNames(value);
}
function Symbols(value) {
  return Object.getOwnPropertySymbols(value);
}
function Values(value) {
  return Object.values(value);
}
function DeepEqualObject(left, right) {
  if (!IsObject(right))
    return false;
  const keys = Keys(left);
  return IsEqual(keys.length, Keys(right).length) && keys.every((key) => IsDeepEqual(left[key], right[key]));
}
function DeepEqualArray(left, right) {
  return IsArray(right) && IsEqual(left.length, right.length) && left.every((_, index) => IsDeepEqual(left[index], right[index]));
}
function IsDeepEqual(left, right) {
  return IsArray(left) ? DeepEqualArray(left, right) : IsObject(left) ? DeepEqualObject(left, right) : IsEqual(left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/emit.mjs
var identifierRegExp = /^[\p{ID_Start}_$][\p{ID_Continue}_$\u200C\u200D]*$/u;
function IsIdentifier(value) {
  return identifierRegExp.test(value);
}
function And(left, right) {
  return `(${left} && ${right})`;
}
function Or(left, right) {
  return `(${left} || ${right})`;
}
function Not(expr) {
  return `!(${expr})`;
}
function IsArray2(value) {
  return `Array.isArray(${value})`;
}
function IsAsyncIterator2(value) {
  return `Guard.IsAsyncIterator(${value})`;
}
function IsBigInt2(value) {
  return `typeof ${value} === "bigint"`;
}
function IsBoolean2(value) {
  return `typeof ${value} === "boolean"`;
}
function IsInteger2(value) {
  return `Number.isInteger(${value})`;
}
function IsIterator2(value) {
  return `Guard.IsIterator(${value})`;
}
function IsNull2(value) {
  return `${value} === null`;
}
function IsNumber2(value) {
  return `Number.isFinite(${value})`;
}
function IsObjectNotArray2(value) {
  return And(IsObject2(value), Not(IsArray2(value)));
}
function IsObject2(value) {
  return `typeof ${value} === "object" && ${value} !== null`;
}
function IsString2(value) {
  return `typeof ${value} === "string"`;
}
function IsSymbol2(value) {
  return `typeof ${value} === "symbol"`;
}
function IsUndefined2(value) {
  return `${value} === undefined`;
}
function IsFunction2(value) {
  return `typeof ${value} === "function"`;
}
function IsConstructor2(value) {
  return `Guard.IsConstructor(${value})`;
}
function IsEqual2(left, right) {
  return `${left} === ${right}`;
}
function IsGreaterThan2(left, right) {
  return `${left} > ${right}`;
}
function IsLessThan2(left, right) {
  return `${left} < ${right}`;
}
function IsLessEqualThan2(left, right) {
  return `${left} <= ${right}`;
}
function IsGreaterEqualThan2(left, right) {
  return `${left} >= ${right}`;
}
function IsMinLength3(value, length) {
  return `Guard.IsMinLength(${value}, ${length})`;
}
function IsMaxLength3(value, length) {
  return `Guard.IsMaxLength(${value}, ${length})`;
}
function Every2(value, offset, params, expression) {
  return IsEqual(offset, "0") ? `${value}.every((${params[0]}, ${params[1]}) => ${expression})` : `((value, callback) => { for(let index = ${offset}; index < value.length; index++) if (!callback(value[index], index)) return false; return true })(${value}, (${params[0]}, ${params[1]}) => ${expression})`;
}
function Entries2(value) {
  return `Object.entries(${value})`;
}
function Keys2(value) {
  return `Object.getOwnPropertyNames(${value})`;
}
function HasPropertyKey2(value, key) {
  const isProtoField = IsEqual(key, '"__proto__"') || IsEqual(key, '"constructor"');
  return isProtoField ? `Object.prototype.hasOwnProperty.call(${value}, ${key})` : `${key} in ${value}`;
}
function IsDeepEqual2(left, right) {
  return `Guard.IsDeepEqual(${left}, ${right})`;
}
function ArrayLiteral(elements) {
  return `[${elements.join(", ")}]`;
}
function ArrowFunction(parameters, body) {
  return `((${parameters.join(", ")}) => ${body})`;
}
function Call(value, arguments_) {
  return `${value}(${arguments_.join(", ")})`;
}
function New(value, arguments_) {
  return `new ${value}(${arguments_.join(", ")})`;
}
function Member(left, right) {
  return `${left}${IsIdentifier(right) ? `.${right}` : `[${Constant(right)}]`}`;
}
function Constant(value) {
  return IsString(value) ? JSON.stringify(value) : `${value}`;
}
function Ternary(condition, true_, false_) {
  return `(${condition} ? ${true_} : ${false_})`;
}
function Statements(statements) {
  return `{ ${statements.join("; ")}; }`;
}
function ConstDeclaration(identifier, expression) {
  return `const ${identifier} = ${expression}`;
}
function If(condition, then) {
  return `if(${condition}) { ${then} }`;
}
function Return(expression) {
  return `return ${expression}`;
}
function ReduceAnd(operands) {
  return IsEqual(operands.length, 0) ? "true" : operands.reduce((left, right) => And(left, right));
}
function ReduceOr(operands) {
  return IsEqual(operands.length, 0) ? "false" : operands.reduce((left, right) => Or(left, right));
}
function PrefixIncrement(expression) {
  return `++${expression}`;
}
function MultipleOf(dividend, divisor) {
  return `Guard.IsMultipleOf(${dividend}, ${divisor})`;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/guard/globals.mjs
var exports_globals = {};
__export(exports_globals, {
  IsUint8ClampedArray: () => IsUint8ClampedArray,
  IsUint8Array: () => IsUint8Array,
  IsUint32Array: () => IsUint32Array,
  IsUint16Array: () => IsUint16Array,
  IsTypeArray: () => IsTypeArray,
  IsString: () => IsString3,
  IsSet: () => IsSet,
  IsRegExp: () => IsRegExp,
  IsNumber: () => IsNumber3,
  IsMap: () => IsMap,
  IsInt8Array: () => IsInt8Array,
  IsInt32Array: () => IsInt32Array,
  IsInt16Array: () => IsInt16Array,
  IsFloat64Array: () => IsFloat64Array,
  IsFloat32Array: () => IsFloat32Array,
  IsDate: () => IsDate,
  IsBoolean: () => IsBoolean3,
  IsBigUint64Array: () => IsBigUint64Array,
  IsBigInt64Array: () => IsBigInt64Array
});
function IsBoolean3(value) {
  return value instanceof Boolean;
}
function IsNumber3(value) {
  return value instanceof Number;
}
function IsString3(value) {
  return value instanceof String;
}
function IsTypeArray(value) {
  return globalThis.ArrayBuffer.isView(value);
}
function IsInt8Array(value) {
  return value instanceof globalThis.Int8Array;
}
function IsUint8Array(value) {
  return value instanceof globalThis.Uint8Array;
}
function IsUint8ClampedArray(value) {
  return value instanceof globalThis.Uint8ClampedArray;
}
function IsInt16Array(value) {
  return value instanceof globalThis.Int16Array;
}
function IsUint16Array(value) {
  return value instanceof globalThis.Uint16Array;
}
function IsInt32Array(value) {
  return value instanceof globalThis.Int32Array;
}
function IsUint32Array(value) {
  return value instanceof globalThis.Uint32Array;
}
function IsFloat32Array(value) {
  return value instanceof globalThis.Float32Array;
}
function IsFloat64Array(value) {
  return value instanceof globalThis.Float64Array;
}
function IsBigInt64Array(value) {
  return value instanceof globalThis.BigInt64Array;
}
function IsBigUint64Array(value) {
  return value instanceof globalThis.BigUint64Array;
}
function IsRegExp(value) {
  return value instanceof globalThis.RegExp;
}
function IsDate(value) {
  return value instanceof globalThis.Date;
}
function IsSet(value) {
  return value instanceof globalThis.Set;
}
function IsMap(value) {
  return value instanceof globalThis.Map;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/_guard.mjs
function IsGuardInterface(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "check") && exports_guard.HasPropertyKey(value, "errors") && exports_guard.IsFunction(value.check) && exports_guard.IsFunction(value.errors);
}
function IsGuard(value) {
  return exports_guard.HasPropertyKey(value, "~guard") && IsGuardInterface(value["~guard"]);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/_refine.mjs
function IsRefine(value) {
  return exports_guard.HasPropertyKey(value, "~refine") && exports_guard.IsArray(value["~refine"]) && exports_guard.Every(value["~refine"], 0, (value2) => exports_guard.IsObject(value2) && exports_guard.HasPropertyKey(value2, "check") && exports_guard.HasPropertyKey(value2, "error") && exports_guard.IsFunction(value2.check) && exports_guard.IsFunction(value2.error));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/schema.mjs
function IsSchemaObject(value) {
  return exports_guard.IsObject(value) && !exports_guard.IsArray(value);
}
function IsBooleanSchema(value) {
  return exports_guard.IsBoolean(value);
}
function IsSchema(value) {
  return IsSchemaObject(value) || IsBooleanSchema(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/additionalItems.mjs
function IsAdditionalItems(schema) {
  return exports_guard.HasPropertyKey(schema, "additionalItems") && IsSchema(schema.additionalItems);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/additionalProperties.mjs
function IsAdditionalProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "additionalProperties") && IsSchema(schema.additionalProperties);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/allOf.mjs
function IsAllOf(schema) {
  return exports_guard.HasPropertyKey(schema, "allOf") && exports_guard.IsArray(schema.allOf) && schema.allOf.every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/anchor.mjs
function IsAnchor(schema) {
  return exports_guard.HasPropertyKey(schema, "$anchor") && exports_guard.IsString(schema.$anchor);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/anyOf.mjs
function IsAnyOf(schema) {
  return exports_guard.HasPropertyKey(schema, "anyOf") && exports_guard.IsArray(schema.anyOf) && schema.anyOf.every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/const.mjs
function IsConst(value) {
  return exports_guard.HasPropertyKey(value, "const");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/contains.mjs
function IsContains(schema) {
  return exports_guard.HasPropertyKey(schema, "contains") && IsSchema(schema.contains);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/default.mjs
function IsDefault(schema) {
  return exports_guard.HasPropertyKey(schema, "default");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/dependencies.mjs
function IsDependencies(schema) {
  return exports_guard.HasPropertyKey(schema, "dependencies") && exports_guard.IsObject(schema.dependencies) && Object.values(schema.dependencies).every((value) => IsSchema(value) || exports_guard.IsArray(value) && value.every((value2) => exports_guard.IsString(value2)));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/dependentRequired.mjs
function IsDependentRequired(schema) {
  return exports_guard.HasPropertyKey(schema, "dependentRequired") && exports_guard.IsObject(schema.dependentRequired) && Object.values(schema.dependentRequired).every((value) => exports_guard.IsArray(value) && value.every((value2) => exports_guard.IsString(value2)));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/dependentSchemas.mjs
function IsDependentSchemas(schema) {
  return exports_guard.HasPropertyKey(schema, "dependentSchemas") && exports_guard.IsObject(schema.dependentSchemas) && Object.values(schema.dependentSchemas).every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/dynamicAnchor.mjs
function IsDynamicAnchor(schema) {
  return exports_guard.HasPropertyKey(schema, "$dynamicAnchor") && exports_guard.IsString(schema.$dynamicAnchor);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/dynamicRef.mjs
function IsDynamicRef(schema) {
  return exports_guard.HasPropertyKey(schema, "$dynamicRef") && exports_guard.IsString(schema.$dynamicRef);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/else.mjs
function IsElse(schema) {
  return exports_guard.HasPropertyKey(schema, "else") && IsSchema(schema.else);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/enum.mjs
function IsEnum(schema) {
  return exports_guard.HasPropertyKey(schema, "enum") && exports_guard.IsArray(schema.enum);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/exclusiveMaximum.mjs
function IsExclusiveMaximum(schema) {
  return exports_guard.HasPropertyKey(schema, "exclusiveMaximum") && (exports_guard.IsNumber(schema.exclusiveMaximum) || exports_guard.IsBigInt(schema.exclusiveMaximum));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/exclusiveMinimum.mjs
function IsExclusiveMinimum(schema) {
  return exports_guard.HasPropertyKey(schema, "exclusiveMinimum") && (exports_guard.IsNumber(schema.exclusiveMinimum) || exports_guard.IsBigInt(schema.exclusiveMinimum));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/format.mjs
function IsFormat(schema) {
  return exports_guard.HasPropertyKey(schema, "format") && exports_guard.IsString(schema.format);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/id.mjs
function IsId(schema) {
  return exports_guard.HasPropertyKey(schema, "$id") && exports_guard.IsString(schema.$id);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/if.mjs
function IsIf(schema) {
  return exports_guard.HasPropertyKey(schema, "if") && IsSchema(schema.if);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/items.mjs
function IsItems(schema) {
  return exports_guard.HasPropertyKey(schema, "items") && (IsSchema(schema.items) || exports_guard.IsArray(schema.items) && schema.items.every((value) => {
    return IsSchema(value);
  }));
}
function IsItemsSized(schema) {
  return IsItems(schema) && exports_guard.IsArray(schema.items);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/maximum.mjs
function IsMaximum(schema) {
  return exports_guard.HasPropertyKey(schema, "maximum") && (exports_guard.IsNumber(schema.maximum) || exports_guard.IsBigInt(schema.maximum));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/maxContains.mjs
function IsMaxContains(schema) {
  return exports_guard.HasPropertyKey(schema, "maxContains") && exports_guard.IsNumber(schema.maxContains);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/maxItems.mjs
function IsMaxItems(schema) {
  return exports_guard.HasPropertyKey(schema, "maxItems") && exports_guard.IsNumber(schema.maxItems);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/maxLength.mjs
function IsMaxLength4(schema) {
  return exports_guard.HasPropertyKey(schema, "maxLength") && exports_guard.IsNumber(schema.maxLength);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/maxProperties.mjs
function IsMaxProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "maxProperties") && exports_guard.IsNumber(schema.maxProperties);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/minimum.mjs
function IsMinimum(schema) {
  return exports_guard.HasPropertyKey(schema, "minimum") && (exports_guard.IsNumber(schema.minimum) || exports_guard.IsBigInt(schema.minimum));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/minContains.mjs
function IsMinContains(schema) {
  return exports_guard.HasPropertyKey(schema, "minContains") && exports_guard.IsNumber(schema.minContains);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/minItems.mjs
function IsMinItems(schema) {
  return exports_guard.HasPropertyKey(schema, "minItems") && exports_guard.IsNumber(schema.minItems);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/minLength.mjs
function IsMinLength4(schema) {
  return exports_guard.HasPropertyKey(schema, "minLength") && exports_guard.IsNumber(schema.minLength);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/minProperties.mjs
function IsMinProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "minProperties") && exports_guard.IsNumber(schema.minProperties);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/multipleOf.mjs
function IsMultipleOf2(schema) {
  return exports_guard.HasPropertyKey(schema, "multipleOf") && (exports_guard.IsNumber(schema.multipleOf) || exports_guard.IsBigInt(schema.multipleOf));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/not.mjs
function IsNot(schema) {
  return exports_guard.HasPropertyKey(schema, "not") && IsSchema(schema.not);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/oneOf.mjs
function IsOneOf(schema) {
  return exports_guard.HasPropertyKey(schema, "oneOf") && exports_guard.IsArray(schema.oneOf) && schema.oneOf.every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/pattern.mjs
function IsPattern(schema) {
  return exports_guard.HasPropertyKey(schema, "pattern") && (exports_guard.IsString(schema.pattern) || schema.pattern instanceof RegExp);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/patternProperties.mjs
function IsPatternProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "patternProperties") && exports_guard.IsObject(schema.patternProperties) && Object.values(schema.patternProperties).every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/prefixItems.mjs
function IsPrefixItems(schema) {
  return exports_guard.HasPropertyKey(schema, "prefixItems") && exports_guard.IsArray(schema.prefixItems) && schema.prefixItems.every((schema2) => IsSchema(schema2));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/properties.mjs
function IsProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "properties") && exports_guard.IsObject(schema.properties) && Object.values(schema.properties).every((value) => IsSchema(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/propertyNames.mjs
function IsPropertyNames(schema) {
  return exports_guard.HasPropertyKey(schema, "propertyNames") && (exports_guard.IsObject(schema.propertyNames) || IsSchema(schema.propertyNames));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/recursiveAnchor.mjs
function IsRecursiveAnchor(schema) {
  return exports_guard.HasPropertyKey(schema, "$recursiveAnchor") && exports_guard.IsBoolean(schema.$recursiveAnchor);
}
function IsRecursiveAnchorTrue(schema) {
  return IsRecursiveAnchor(schema) && exports_guard.IsEqual(schema.$recursiveAnchor, true);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/recursiveRef.mjs
function IsRecursiveRef(schema) {
  return exports_guard.HasPropertyKey(schema, "$recursiveRef") && exports_guard.IsString(schema.$recursiveRef);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/ref.mjs
function IsRef(schema) {
  return exports_guard.HasPropertyKey(schema, "$ref") && exports_guard.IsString(schema.$ref);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/required.mjs
function IsRequired(schema) {
  return exports_guard.HasPropertyKey(schema, "required") && exports_guard.IsArray(schema.required) && schema.required.every((value) => exports_guard.IsString(value));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/then.mjs
function IsThen(schema) {
  return exports_guard.HasPropertyKey(schema, "then") && IsSchema(schema.then);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/type.mjs
function IsType(schema) {
  return exports_guard.HasPropertyKey(schema, "type") && (exports_guard.IsString(schema.type) || exports_guard.IsArray(schema.type) && schema.type.every((value) => exports_guard.IsString(value)));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/uniqueItems.mjs
function IsUniqueItems(schema) {
  return exports_guard.HasPropertyKey(schema, "uniqueItems") && exports_guard.IsBoolean(schema.uniqueItems);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/unevaluatedItems.mjs
function IsUnevaluatedItems(schema) {
  return exports_guard.HasPropertyKey(schema, "unevaluatedItems") && IsSchema(schema.unevaluatedItems);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/types/unevaluatedProperties.mjs
function IsUnevaluatedProperties(schema) {
  return exports_guard.HasPropertyKey(schema, "unevaluatedProperties") && IsSchema(schema.unevaluatedProperties);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_context.mjs
function HasUnevaluatedFromObject(value) {
  return IsUnevaluatedItems(value) || IsUnevaluatedProperties(value) || exports_guard.Keys(value).some((key) => HasUnevaluatedFromUnknown(value[key]));
}
function HasUnevaluatedFromArray(value) {
  return value.some((value2) => HasUnevaluatedFromUnknown(value2));
}
function HasUnevaluatedFromUnknown(value) {
  return exports_guard.IsArray(value) ? HasUnevaluatedFromArray(value) : exports_guard.IsObject(value) ? HasUnevaluatedFromObject(value) : false;
}
function HasUnevaluated(context, schema2) {
  return HasUnevaluatedFromUnknown(schema2) || exports_guard.Keys(context).some((key) => HasUnevaluatedFromUnknown(context[key]));
}

class BuildContext {
  constructor(hasUnevaluated) {
    this.hasUnevaluated = hasUnevaluated;
  }
  UseUnevaluated() {
    return this.hasUnevaluated;
  }
  Push() {
    return exports_emit.Call(exports_emit.Member("context", "Push"), []);
  }
  Pop() {
    return exports_emit.Call(exports_emit.Member("context", "Pop"), []);
  }
  AddIndex(index) {
    return exports_emit.Call(exports_emit.Member("context", "AddIndex"), [index]);
  }
  AddKey(key) {
    return exports_emit.Call(exports_emit.Member("context", "AddKey"), [key]);
  }
  Merge(results) {
    return exports_emit.Call(exports_emit.Member("context", "Merge"), [results]);
  }
}

class CheckContext {
  constructor() {
    const indices = new Set;
    const keys = new Set;
    this.stack = [{ indices, keys }];
  }
  Push() {
    const indices = new Set;
    const keys = new Set;
    this.stack.push({ indices, keys });
    return true;
  }
  Pop() {
    this.stack.pop();
    return true;
  }
  AddIndex(index) {
    this.GetIndices().add(index);
    return true;
  }
  AddKey(key) {
    this.GetKeys().add(key);
    return true;
  }
  GetIndices() {
    const top = this.stack[this.stack.length - 1];
    return top.indices;
  }
  GetKeys() {
    const top = this.stack[this.stack.length - 1];
    return top.keys;
  }
  Merge(results) {
    for (const context of results) {
      context.GetIndices().forEach((value) => this.GetIndices().add(value));
      context.GetKeys().forEach((value) => this.GetKeys().add(value));
    }
    return true;
  }
}

class ErrorContext extends CheckContext {
  constructor(callback) {
    super();
    this.callback = callback;
  }
  AddError(error) {
    this.callback(error);
    return false;
  }
}

class AccumulatedErrorContext extends ErrorContext {
  constructor() {
    super((error) => this.errors.push(error));
    this.errors = [];
  }
  AddError(error) {
    this.errors.push(error);
    return false;
  }
  GetErrors() {
    return this.errors;
  }
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_externals.mjs
var state = {
  identifier: "External",
  variables: []
};
function CreateVariable(value) {
  const call = `External[${state.variables.length}]`;
  state.variables.push(value);
  return call;
}
function ResetExternal() {
  state.variables = [];
}
function GetExternal() {
  return { ...state };
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_guard.mjs
function BuildGuard(_stack, _context, schema2, value) {
  return exports_emit.Call(exports_emit.Member(exports_emit.Member(CreateVariable(schema2), "~guard"), "check"), [value]);
}
function CheckGuard(_stack, _context, schema2, value) {
  return schema2["~guard"].check(value);
}
function ErrorGuard(_stack, context, schemaPath, instancePath, schema2, value) {
  return schema2["~guard"].check(value) || context.AddError({
    keyword: "~guard",
    schemaPath,
    instancePath,
    params: { errors: schema2["~guard"].errors(value) }
  });
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/hashing/hash.mjs
var exports_hash = {};
__export(exports_hash, {
  HashCode: () => HashCode,
  Hash: () => Hash
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/unreachable/unreachable.mjs
function Unreachable() {
  throw new Error("Unreachable");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/hashing/hash.mjs
function InstanceKeys(value) {
  const propertyKeys = new Set;
  let current = value;
  while (current && current !== Object.prototype) {
    for (const key of Reflect.ownKeys(current)) {
      if (key !== "constructor" && typeof key !== "symbol")
        propertyKeys.add(key);
    }
    current = Object.getPrototypeOf(current);
  }
  return [...propertyKeys];
}
function IsIEEE754(value) {
  return typeof value === "number";
}
var ByteMarker;
(function(ByteMarker2) {
  ByteMarker2[ByteMarker2["Array"] = 0] = "Array";
  ByteMarker2[ByteMarker2["BigInt"] = 1] = "BigInt";
  ByteMarker2[ByteMarker2["Boolean"] = 2] = "Boolean";
  ByteMarker2[ByteMarker2["Date"] = 3] = "Date";
  ByteMarker2[ByteMarker2["Constructor"] = 4] = "Constructor";
  ByteMarker2[ByteMarker2["Function"] = 5] = "Function";
  ByteMarker2[ByteMarker2["Null"] = 6] = "Null";
  ByteMarker2[ByteMarker2["Number"] = 7] = "Number";
  ByteMarker2[ByteMarker2["Object"] = 8] = "Object";
  ByteMarker2[ByteMarker2["RegExp"] = 9] = "RegExp";
  ByteMarker2[ByteMarker2["String"] = 10] = "String";
  ByteMarker2[ByteMarker2["Symbol"] = 11] = "Symbol";
  ByteMarker2[ByteMarker2["TypeArray"] = 12] = "TypeArray";
  ByteMarker2[ByteMarker2["Undefined"] = 13] = "Undefined";
})(ByteMarker || (ByteMarker = {}));
var Accumulator = BigInt("14695981039346656037");
var [Prime, Size] = [BigInt("1099511628211"), BigInt("18446744073709551616")];
var Bytes = Array.from({ length: 256 }).map((_, i) => BigInt(i));
var F64 = new Float64Array(1);
var F64In = new DataView(F64.buffer);
var F64Out = new Uint8Array(F64.buffer);
function FNV1A64_OP(byte) {
  Accumulator = Accumulator ^ Bytes[byte];
  Accumulator = Accumulator * Prime % Size;
}
function FromArray(value) {
  FNV1A64_OP(ByteMarker.Array);
  for (const item of value) {
    FromValue(item);
  }
}
function FromBigInt(value) {
  FNV1A64_OP(ByteMarker.BigInt);
  F64In.setBigInt64(0, value);
  for (const byte of F64Out) {
    FNV1A64_OP(byte);
  }
}
function FromBoolean(value) {
  FNV1A64_OP(ByteMarker.Boolean);
  FNV1A64_OP(value ? 1 : 0);
}
function FromConstructor(value) {
  FNV1A64_OP(ByteMarker.Constructor);
  FromValue(value.toString());
}
function FromDate(value) {
  FNV1A64_OP(ByteMarker.Date);
  FromValue(value.getTime());
}
function FromFunction(value) {
  FNV1A64_OP(ByteMarker.Function);
  FromValue(value.toString());
}
function FromNull(_value) {
  FNV1A64_OP(ByteMarker.Null);
}
function FromNumber(value) {
  FNV1A64_OP(ByteMarker.Number);
  F64In.setFloat64(0, value, true);
  for (const byte of F64Out) {
    FNV1A64_OP(byte);
  }
}
function FromObject(value) {
  FNV1A64_OP(ByteMarker.Object);
  for (const key of InstanceKeys(value).sort()) {
    FromValue(key);
    FromValue(value[key]);
  }
}
function FromRegExp(value) {
  FNV1A64_OP(ByteMarker.RegExp);
  FromString(value.toString());
}
var encoder = new TextEncoder;
function FromString(value) {
  FNV1A64_OP(ByteMarker.String);
  for (const byte of encoder.encode(value)) {
    FNV1A64_OP(byte);
  }
}
function FromSymbol(value) {
  FNV1A64_OP(ByteMarker.Symbol);
  FromValue(value.toString());
}
function FromTypeArray(value) {
  FNV1A64_OP(ByteMarker.TypeArray);
  const buffer = new Uint8Array(value.buffer);
  for (let i = 0;i < buffer.length; i++) {
    FNV1A64_OP(buffer[i]);
  }
}
function FromUndefined(_value) {
  return FNV1A64_OP(ByteMarker.Undefined);
}
function FromValue(value) {
  return exports_globals.IsTypeArray(value) ? FromTypeArray(value) : exports_globals.IsDate(value) ? FromDate(value) : exports_globals.IsRegExp(value) ? FromRegExp(value) : exports_globals.IsBoolean(value) ? FromBoolean(value.valueOf()) : exports_globals.IsString(value) ? FromString(value.valueOf()) : exports_globals.IsNumber(value) ? FromNumber(value.valueOf()) : IsIEEE754(value) ? FromNumber(value) : exports_guard.IsArray(value) ? FromArray(value) : exports_guard.IsBoolean(value) ? FromBoolean(value) : exports_guard.IsBigInt(value) ? FromBigInt(value) : exports_guard.IsConstructor(value) ? FromConstructor(value) : exports_guard.IsNull(value) ? FromNull(value) : exports_guard.IsObject(value) ? FromObject(value) : exports_guard.IsString(value) ? FromString(value) : exports_guard.IsSymbol(value) ? FromSymbol(value) : exports_guard.IsUndefined(value) ? FromUndefined(value) : exports_guard.IsFunction(value) ? FromFunction(value) : Unreachable();
}
function HashCode(value) {
  Accumulator = BigInt("14695981039346656037");
  FromValue(value);
  return Accumulator;
}
function Hash(value) {
  return HashCode(value).toString(16).padStart(16, "0");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_refine.mjs
function BuildRefine(_stack, _context, schema2, value) {
  const refinements = CreateVariable(schema2["~refine"].map((refinement) => refinement));
  return exports_emit.Every(refinements, exports_emit.Constant(0), ["refinement", "_"], exports_emit.Call(exports_emit.Member("refinement", "check"), [value]));
}
function CheckRefine(_stack, _context, schema2, value) {
  return exports_guard.Every(schema2["~refine"], 0, (refinement, _) => refinement.check(value));
}
function ErrorRefine(_stack, context, schemaPath, instancePath, schema2, value) {
  return exports_guard.EveryAll(schema2["~refine"], 0, (refinement, index) => {
    return refinement.check(value) || context.AddError({
      keyword: "~refine",
      schemaPath,
      instancePath,
      params: { index, message: refinement.error(value) }
    });
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_unique.mjs
var index = 0;
function Unique() {
  return `var_${index++}`;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/additionalItems.mjs
function IsValid(schema2) {
  return IsItems(schema2) && exports_guard.IsArray(schema2.items);
}
function BuildAdditionalItems(stack, context, schema2, value) {
  if (!IsValid(schema2))
    return exports_emit.Constant(true);
  const [item, index2] = [Unique(), Unique()];
  const isSchema = BuildSchemaPushStack(stack, context, schema2.additionalItems, item);
  const isLength = exports_emit.IsLessThan(index2, exports_emit.Constant(schema2.items.length));
  const addIndex = context.AddIndex(index2);
  const guarded = context.UseUnevaluated() ? exports_emit.Or(isLength, exports_emit.And(isSchema, addIndex)) : exports_emit.Or(isLength, isSchema);
  return exports_emit.Call(exports_emit.Member(value, "every"), [exports_emit.ArrowFunction([item, index2], guarded)]);
}
function CheckAdditionalItems(stack, context, schema2, value) {
  if (!IsValid(schema2))
    return true;
  const isAdditionalItems = value.every((item, index2) => {
    return exports_guard.IsLessThan(index2, schema2.items.length) || CheckSchemaPushStack(stack, context, schema2.additionalItems, item) && context.AddIndex(index2);
  });
  return isAdditionalItems;
}
function ErrorAdditionalItems(stack, context, schemaPath, instancePath, schema2, value) {
  if (!IsValid(schema2))
    return true;
  const isAdditionalItems = value.every((item, index2) => {
    const nextSchemaPath = `${schemaPath}/additionalItems`;
    const nextInstancePath = `${instancePath}/${index2}`;
    return exports_guard.IsLessThan(index2, schema2.items.length) || ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema2.additionalItems, item) && context.AddIndex(index2);
  });
  return isAdditionalItems;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/additionalProperties.mjs
function GetPropertyKeyAsPattern(key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return `^${escaped}$`;
}
function GetPropertiesPattern(schema2) {
  const patterns = [];
  if (IsPatternProperties(schema2))
    patterns.push(...exports_guard.Keys(schema2.patternProperties));
  if (IsProperties(schema2))
    patterns.push(...exports_guard.Keys(schema2.properties).map(GetPropertyKeyAsPattern));
  return exports_guard.IsEqual(patterns.length, 0) ? "(?!)" : `(${patterns.join("|")})`;
}
function CanAdditionalPropertiesFast(_context, schema2, _value) {
  return IsRequired(schema2) && IsProperties(schema2) && !IsPatternProperties(schema2) && exports_guard.IsEqual(schema2.additionalProperties, false) && exports_guard.IsEqual(exports_guard.Keys(schema2.properties).length, schema2.required.length);
}
function BuildAdditionalPropertiesFast(_context, schema2, value) {
  return exports_emit.IsEqual(exports_emit.Member(exports_emit.Call(exports_emit.Member("Object", "getOwnPropertyNames"), [value]), "length"), exports_emit.Constant(schema2.required.length));
}
function BuildAdditionalPropertiesStandard(stack, context, schema2, value) {
  const [key, _index] = [Unique(), Unique()];
  const regexp = CreateVariable(new RegExp(GetPropertiesPattern(schema2)));
  const isSchema = BuildSchemaPushStack(stack, context, schema2.additionalProperties, `${value}[${key}]`);
  const isKey = exports_emit.Call(exports_emit.Member(regexp, "test"), [key]);
  const addKey = context.AddKey(key);
  const guarded = context.UseUnevaluated() ? exports_emit.Or(isKey, exports_emit.And(isSchema, addKey)) : exports_emit.Or(isKey, isSchema);
  const result = exports_emit.Every(exports_emit.Keys(value), exports_emit.Constant(0), [key, _index], guarded);
  return result;
}
function BuildAdditionalProperties(stack, context, schema2, value) {
  return CanAdditionalPropertiesFast(context, schema2, value) ? BuildAdditionalPropertiesFast(context, schema2, value) : BuildAdditionalPropertiesStandard(stack, context, schema2, value);
}
function CheckAdditionalProperties(stack, context, schema2, value) {
  const regexp = new RegExp(GetPropertiesPattern(schema2));
  const isAdditionalProperties = exports_guard.Every(exports_guard.Keys(value), 0, (key, _index) => {
    return regexp.test(key) || CheckSchemaPushStack(stack, context, schema2.additionalProperties, value[key]) && context.AddKey(key);
  });
  return isAdditionalProperties;
}
function ErrorAdditionalProperties(stack, context, schemaPath, instancePath, schema2, value) {
  const regexp = new RegExp(GetPropertiesPattern(schema2));
  const additionalProperties2 = [];
  const isAdditionalProperties = exports_guard.EveryAll(exports_guard.Keys(value), 0, (key, _index) => {
    const nextSchemaPath = `${schemaPath}/additionalProperties`;
    const nextInstancePath = `${instancePath}/${key}`;
    const nextContext = new AccumulatedErrorContext;
    const isAdditionalProperty = regexp.test(key) || ErrorSchemaPushStack(stack, nextContext, nextSchemaPath, nextInstancePath, schema2.additionalProperties, value[key]) && context.AddKey(key);
    if (!isAdditionalProperty)
      additionalProperties2.push(key);
    return isAdditionalProperty;
  });
  return isAdditionalProperties || context.AddError({
    keyword: "additionalProperties",
    schemaPath,
    instancePath,
    params: { additionalProperties: additionalProperties2 }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_reducer.mjs
function Reducer(stack, context, schemas, value, check) {
  const results = exports_emit.ConstDeclaration("results", "[]");
  const context_n = schemas.map((_schema, index2) => exports_emit.ConstDeclaration(`context_${index2}`, exports_emit.New("CheckContext", [])));
  const condition_n = schemas.map((schema2, index2) => exports_emit.ConstDeclaration(`condition_${index2}`, exports_emit.Call(exports_emit.ArrowFunction(["context"], BuildSchema(stack, context, schema2, value)), [`context_${index2}`])));
  const checks = schemas.map((_schema, index2) => exports_emit.If(`condition_${index2}`, exports_emit.Call(exports_emit.Member("results", "push"), [`context_${index2}`])));
  const returns = exports_emit.Return(exports_emit.And(check, context.Merge("results")));
  return exports_emit.Call(exports_emit.ArrowFunction([], exports_emit.Statements([results, ...context_n, ...condition_n, ...checks, returns])), []);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/allOf.mjs
function BuildAllOfStandard(stack, context, schema2, value) {
  return Reducer(stack, context, schema2.allOf, value, exports_emit.IsEqual(exports_emit.Member("results", "length"), exports_emit.Constant(schema2.allOf.length)));
}
function BuildAllOfFast(stack, context, schema2, value) {
  return exports_emit.ReduceAnd(schema2.allOf.map((schema3) => BuildSchema(stack, context, schema3, value)));
}
function BuildAllOf(stack, context, schema2, value) {
  return context.UseUnevaluated() ? BuildAllOfStandard(stack, context, schema2, value) : BuildAllOfFast(stack, context, schema2, value);
}
function CheckAllOf(stack, context, schema2, value) {
  const results = schema2.allOf.reduce((result, schema3) => {
    const nextContext = new CheckContext;
    return CheckSchema(stack, nextContext, schema3, value) ? [...result, nextContext] : result;
  }, []);
  return exports_guard.IsEqual(results.length, schema2.allOf.length) && context.Merge(results);
}
function ErrorAllOf(stack, context, schemaPath, instancePath, schema2, value) {
  const failedContexts = [];
  const results = schema2.allOf.reduce((result, schema3, index2) => {
    const nextSchemaPath = `${schemaPath}/allOf/${index2}`;
    const nextContext = new AccumulatedErrorContext;
    const isSchema = ErrorSchema(stack, nextContext, nextSchemaPath, instancePath, schema3, value);
    if (!isSchema)
      failedContexts.push(nextContext);
    return isSchema ? [...result, nextContext] : result;
  }, []);
  const isAllOf = exports_guard.IsEqual(results.length, schema2.allOf.length) && context.Merge(results);
  if (!isAllOf)
    failedContexts.forEach((failed) => failed.GetErrors().forEach((error) => context.AddError(error)));
  return isAllOf;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/anyOf.mjs
function BuildAnyOfStandard(stack, context, schema2, value) {
  return Reducer(stack, context, schema2.anyOf, value, exports_emit.IsGreaterThan(exports_emit.Member("results", "length"), exports_emit.Constant(0)));
}
function BuildAnyOfFast(stack, context, schema2, value) {
  return exports_emit.ReduceOr(schema2.anyOf.map((schema3) => BuildSchema(stack, context, schema3, value)));
}
function BuildAnyOf(stack, context, schema2, value) {
  return context.UseUnevaluated() ? BuildAnyOfStandard(stack, context, schema2, value) : BuildAnyOfFast(stack, context, schema2, value);
}
function CheckAnyOf(stack, context, schema2, value) {
  const results = schema2.anyOf.reduce((result, schema3) => {
    const nextContext = new CheckContext;
    return CheckSchema(stack, nextContext, schema3, value) ? [...result, nextContext] : result;
  }, []);
  return exports_guard.IsGreaterThan(results.length, 0) && context.Merge(results);
}
function ErrorAnyOf(stack, context, schemaPath, instancePath, schema2, value) {
  const failedContexts = [];
  const results = schema2.anyOf.reduce((result, schema3, index2) => {
    const nextContext = new AccumulatedErrorContext;
    const nextSchemaPath = `${schemaPath}/anyOf/${index2}`;
    const isSchema = ErrorSchema(stack, nextContext, nextSchemaPath, instancePath, schema3, value);
    if (!isSchema)
      failedContexts.push(nextContext);
    return isSchema ? [...result, nextContext] : result;
  }, []);
  const isAnyOf = exports_guard.IsGreaterThan(results.length, 0) && context.Merge(results);
  if (!isAnyOf)
    failedContexts.forEach((failed) => failed.GetErrors().forEach((error) => context.AddError(error)));
  return isAnyOf || context.AddError({
    keyword: "anyOf",
    schemaPath,
    instancePath,
    params: {}
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/boolean.mjs
function BuildBooleanSchema(_stack, _context, schema2, _value) {
  return schema2 ? exports_emit.Constant(true) : exports_emit.Constant(false);
}
function CheckBooleanSchema(_stack, _context, schema2, _value) {
  return schema2;
}
function ErrorBooleanSchema(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckBooleanSchema(stack, context, schema2, value) || context.AddError({
    keyword: "boolean",
    schemaPath,
    instancePath,
    params: {}
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/const.mjs
function BuildConst(_stack, _context, schema2, value) {
  return exports_guard.IsValueLike(schema2.const) ? exports_emit.IsEqual(value, exports_emit.Constant(schema2.const)) : exports_emit.IsDeepEqual(value, CreateVariable(schema2.const));
}
function CheckConst(_stack, _context, schema2, value) {
  return exports_guard.IsValueLike(schema2.const) ? exports_guard.IsEqual(value, schema2.const) : exports_guard.IsDeepEqual(value, schema2.const);
}
function ErrorConst(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckConst(stack, context, schema2, value) || context.AddError({
    keyword: "const",
    schemaPath,
    instancePath,
    params: { allowedValue: schema2.const }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/contains.mjs
function IsValid2(schema2) {
  return !(IsMinContains(schema2) && exports_guard.IsEqual(schema2.minContains, 0));
}
function BuildContains(stack, context, schema2, value) {
  if (!IsValid2(schema2))
    return exports_emit.Constant(true);
  const item = Unique();
  const isLength = exports_emit.Not(exports_emit.IsEqual(exports_emit.Member(value, "length"), exports_emit.Constant(0)));
  const isSome = exports_emit.Call(exports_emit.Member(value, "some"), [exports_emit.ArrowFunction([item], BuildSchema(stack, context, schema2.contains, item))]);
  return exports_emit.And(isLength, isSome);
}
function CheckContains(stack, context, schema2, value) {
  if (!IsValid2(schema2))
    return true;
  return !exports_guard.IsEqual(value.length, 0) && value.some((item) => CheckSchema(stack, context, schema2.contains, item));
}
function ErrorContains(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckContains(stack, context, schema2, value) || context.AddError({
    keyword: "contains",
    schemaPath,
    instancePath,
    params: { minContains: 1 }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/dependencies.mjs
function BuildDependencies(stack, context, schema2, value) {
  const isLength = exports_emit.IsEqual(exports_emit.Member(exports_emit.Keys(value), "length"), exports_emit.Constant(0));
  const isEveryDependency = exports_emit.ReduceAnd(exports_guard.Entries(schema2.dependencies).map(([key, schema3]) => {
    const notKey = exports_emit.Not(exports_emit.HasPropertyKey(value, exports_emit.Constant(key)));
    const isSchema = BuildSchema(stack, context, schema3, value);
    const isEveryKey = (schema4) => exports_emit.ReduceAnd(schema4.map((key2) => exports_emit.HasPropertyKey(value, exports_emit.Constant(key2))));
    return exports_emit.Or(notKey, exports_guard.IsArray(schema3) ? isEveryKey(schema3) : isSchema);
  }));
  return exports_emit.Or(isLength, isEveryDependency);
}
function CheckDependencies(stack, context, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEvery = exports_guard.Every(exports_guard.Entries(schema2.dependencies), 0, ([key, schema3]) => {
    return !exports_guard.HasPropertyKey(value, key) || (exports_guard.IsArray(schema3) ? schema3.every((key2) => exports_guard.HasPropertyKey(value, key2)) : CheckSchema(stack, context, schema3, value));
  });
  return isLength || isEvery;
}
function ErrorDependencies(stack, context, schemaPath, instancePath, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEvery = exports_guard.EveryAll(exports_guard.Entries(schema2.dependencies), 0, ([key, schema3]) => {
    const nextSchemaPath = `${schemaPath}/dependencies/${key}`;
    return !exports_guard.HasPropertyKey(value, key) || (exports_guard.IsArray(schema3) ? schema3.every((dependency) => exports_guard.HasPropertyKey(value, dependency) || context.AddError({
      keyword: "dependencies",
      schemaPath,
      instancePath,
      params: { property: key, dependencies: schema3 }
    })) : ErrorSchema(stack, context, nextSchemaPath, instancePath, schema3, value));
  });
  return isLength || isEvery;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/dependentRequired.mjs
function BuildDependentRequired(_stack, _context, schema2, value) {
  const isLength = exports_emit.IsEqual(exports_emit.Member(exports_emit.Keys(value), "length"), exports_emit.Constant(0));
  const isEvery = exports_emit.ReduceAnd(exports_guard.Entries(schema2.dependentRequired).map(([key, keys]) => {
    const notKey = exports_emit.Not(exports_emit.HasPropertyKey(value, exports_emit.Constant(key)));
    const everyKey = exports_emit.ReduceAnd(keys.map((key2) => exports_emit.HasPropertyKey(value, exports_emit.Constant(key2))));
    return exports_emit.Or(notKey, everyKey);
  }));
  return exports_emit.Or(isLength, isEvery);
}
function CheckDependentRequired(_stack, _context, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEvery = exports_guard.Every(exports_guard.Entries(schema2.dependentRequired), 0, ([key, keys]) => {
    return !exports_guard.HasPropertyKey(value, key) || keys.every((key2) => exports_guard.HasPropertyKey(value, key2));
  });
  return isLength || isEvery;
}
function ErrorDependentRequired(_stack, context, schemaPath, instancePath, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEveryEntry = exports_guard.EveryAll(exports_guard.Entries(schema2.dependentRequired), 0, ([key, keys]) => {
    return !exports_guard.HasPropertyKey(value, key) || exports_guard.EveryAll(keys, 0, (dependency) => exports_guard.HasPropertyKey(value, dependency) || context.AddError({
      keyword: "dependentRequired",
      schemaPath,
      instancePath,
      params: { property: key, dependencies: keys }
    }));
  });
  return isLength || isEveryEntry;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/dependentSchemas.mjs
function BuildDependentSchemas(stack, context, schema2, value) {
  const isLength = exports_emit.IsEqual(exports_emit.Member(exports_emit.Keys(value), "length"), exports_emit.Constant(0));
  const isEvery = exports_emit.ReduceAnd(exports_guard.Entries(schema2.dependentSchemas).map(([key, schema3]) => {
    const notKey = exports_emit.Not(exports_emit.HasPropertyKey(value, exports_emit.Constant(key)));
    const isSchema = BuildSchema(stack, context, schema3, value);
    return exports_emit.Or(notKey, isSchema);
  }));
  return exports_emit.Or(isLength, isEvery);
}
function CheckDependentSchemas(stack, context, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEvery = exports_guard.Every(exports_guard.Entries(schema2.dependentSchemas), 0, ([key, schema3]) => {
    return !exports_guard.HasPropertyKey(value, key) || CheckSchema(stack, context, schema3, value);
  });
  return isLength || isEvery;
}
function ErrorDependentSchemas(stack, context, schemaPath, instancePath, schema2, value) {
  const isLength = exports_guard.IsEqual(exports_guard.Keys(value).length, 0);
  const isEvery = exports_guard.EveryAll(exports_guard.Entries(schema2.dependentSchemas), 0, ([key, schema3]) => {
    const nextSchemaPath = `${schemaPath}/dependentSchemas/${key}`;
    return !exports_guard.HasPropertyKey(value, key) || ErrorSchema(stack, context, nextSchemaPath, instancePath, schema3, value);
  });
  return isLength || isEvery;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/dynamicRef.mjs
function BuildDynamicRef(stack, context, schema2, value) {
  const target = stack.DynamicRef(schema2) ?? false;
  return CreateFunction(stack, context, target, value);
}
function CheckDynamicRef(stack, context, schema2, value) {
  const target = stack.DynamicRef(schema2) ?? false;
  return IsSchema(target) && CheckSchema(stack, context, target, value);
}
function ErrorDynamicRef(stack, context, _schemaPath, instancePath, schema2, value) {
  const target = stack.DynamicRef(schema2) ?? false;
  return IsSchema(target) && ErrorSchema(stack, context, "#", instancePath, target, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/enum.mjs
function BuildEnum(_stack, _context, schema2, value) {
  return exports_emit.ReduceOr(schema2.enum.map((option) => {
    if (exports_guard.IsValueLike(option))
      return exports_emit.IsEqual(value, exports_emit.Constant(option));
    const variable = CreateVariable(option);
    return exports_emit.IsDeepEqual(value, variable);
  }));
}
function CheckEnum(_stack, _context, schema2, value) {
  return schema2.enum.some((option) => exports_guard.IsValueLike(option) ? exports_guard.IsEqual(value, option) : exports_guard.IsDeepEqual(value, option));
}
function ErrorEnum(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckEnum(stack, context, schema2, value) || context.AddError({
    keyword: "enum",
    schemaPath,
    instancePath,
    params: { allowedValues: schema2.enum }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/exclusiveMaximum.mjs
function BuildExclusiveMaximum(_stack, _context, schema2, value) {
  return exports_emit.IsLessThan(value, exports_emit.Constant(schema2.exclusiveMaximum));
}
function CheckExclusiveMaximum(_stack, _context, schema2, value) {
  return exports_guard.IsLessThan(value, schema2.exclusiveMaximum);
}
function ErrorExclusiveMaximum(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckExclusiveMaximum(stack, context, schema2, value) || context.AddError({
    keyword: "exclusiveMaximum",
    schemaPath,
    instancePath,
    params: { comparison: "<", limit: schema2.exclusiveMaximum }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/exclusiveMinimum.mjs
function BuildExclusiveMinimum(_stack, _context, schema2, value) {
  return exports_emit.IsGreaterThan(value, exports_emit.Constant(schema2.exclusiveMinimum));
}
function CheckExclusiveMinimum(_stack, _context, schema2, value) {
  return exports_guard.IsGreaterThan(value, schema2.exclusiveMinimum);
}
function ErrorExclusiveMinimum(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckExclusiveMinimum(stack, context, schema2, value) || context.AddError({
    keyword: "exclusiveMinimum",
    schemaPath,
    instancePath,
    params: { comparison: ">", limit: schema2.exclusiveMinimum }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/format.mjs
var exports_format = {};
__export(exports_format, {
  Test: () => Test,
  Set: () => Set2,
  Reset: () => Reset,
  IsUuid: () => IsUuid,
  IsUrl: () => IsUrl,
  IsUriTemplate: () => IsUriTemplate,
  IsUriReference: () => IsUriReference,
  IsUri: () => IsUri,
  IsTime: () => IsTime,
  IsRelativeJsonPointer: () => IsRelativeJsonPointer,
  IsRegex: () => IsRegex,
  IsJsonPointerUriFragment: () => IsJsonPointerUriFragment,
  IsJsonPointer: () => IsJsonPointer,
  IsIriReference: () => IsIriReference,
  IsIri: () => IsIri,
  IsIdnHostname: () => IsIdnHostname,
  IsIdnEmail: () => IsIdnEmail,
  IsIPv6: () => IsIPv6,
  IsIPv4: () => IsIPv4,
  IsHostname: () => IsHostname,
  IsEmail: () => IsEmail,
  IsDuration: () => IsDuration,
  IsDateTime: () => IsDateTime,
  IsDate: () => IsDate2,
  Has: () => Has,
  Get: () => Get,
  Entries: () => Entries3,
  Clear: () => Clear
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/date.mjs
var DAYS = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
var DATE = /^(\d\d\d\d)-(\d\d)-(\d\d)$/;
function IsLeapYear(year) {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}
function IsDate2(value) {
  const matches = DATE.exec(value);
  if (!matches)
    return false;
  const year = +matches[1];
  const month = +matches[2];
  const day = +matches[3];
  return month >= 1 && month <= 12 && day >= 1 && day <= (month === 2 && IsLeapYear(year) ? 29 : DAYS[month]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/time.mjs
var TIME = /^(\d\d):(\d\d):(\d\d(?:\.\d+)?)(?:Z|([+-])(\d\d):(\d\d))?$/i;
function IsTime(value, strictTimeZone = true) {
  const matches = TIME.exec(value);
  if (!matches)
    return false;
  const hr = +matches[1];
  const min = +matches[2];
  const sec = +matches[3];
  const tzSign = matches[4] === "-" ? -1 : 1;
  const tzH = +(matches[5] || 0);
  const tzM = +(matches[6] || 0);
  if (tzH > 23 || tzM > 59)
    return false;
  if (strictTimeZone && !matches[4] && value.toLowerCase().indexOf("z") === -1) {
    return false;
  }
  if (hr <= 23 && min <= 59 && sec < 60)
    return true;
  const utcMin = min - tzM * tzSign;
  const utcHr = hr - tzH * tzSign - (utcMin < 0 ? 1 : 0);
  return (utcHr === 23 || utcHr === -1) && (utcMin === 59 || utcMin === -1) && sec < 61;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/date_time.mjs
function IsDateTime(value, strictTimeZone = true) {
  const dateTime = value.split(/T/i);
  return dateTime.length === 2 && IsDate2(dateTime[0]) && IsTime(dateTime[1], strictTimeZone);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/duration.mjs
var Duration = /^P((\d+Y(\d+M(\d+D)?)?|\d+M(\d+D)?|\d+D)(T(\d+H(\d+M(\d+S)?)?|\d+M(\d+S)?|\d+S))?|T(\d+H(\d+M(\d+S)?)?|\d+M(\d+S)?|\d+S)|\d+W)$/;
function IsDuration(value) {
  return Duration.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/email.mjs
var Email = /^(?!.*\.\.)[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*$/i;
function IsEmail(value) {
  return Email.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/_puny.mjs
var PUNYCODE_BASE = 36;
var PUNYCODE_TMIN = 1;
var PUNYCODE_TMAX = 26;
var PUNYCODE_SKEW = 38;
var PUNYCODE_DAMP = 700;
var PUNYCODE_INITIAL_BIAS = 72;
var PUNYCODE_INITIAL_N = 128;
function Adapt(delta, numPoints, firstTime) {
  delta = firstTime ? Math.floor(delta / PUNYCODE_DAMP) : delta >> 1;
  delta += Math.floor(delta / numPoints);
  let k = 0;
  while (delta > (PUNYCODE_BASE - PUNYCODE_TMIN) * PUNYCODE_TMAX >> 1) {
    delta = Math.floor(delta / (PUNYCODE_BASE - PUNYCODE_TMIN));
    k += PUNYCODE_BASE;
  }
  return k + Math.floor((PUNYCODE_BASE - PUNYCODE_TMIN + 1) * delta / (delta + PUNYCODE_SKEW));
}
function Decode(value) {
  const output = [];
  let n = PUNYCODE_INITIAL_N;
  let i = 0;
  let bias = PUNYCODE_INITIAL_BIAS;
  const delimIdx = value.lastIndexOf("-");
  if (delimIdx > 0) {
    for (let j = 0;j < delimIdx; j++) {
      const cp = value.charCodeAt(j);
      if (cp >= 128)
        throw new Error("Invalid punycode: non-basic before delimiter");
      output.push(cp);
    }
  }
  let inIdx = delimIdx < 0 ? 0 : delimIdx + 1;
  while (inIdx < value.length) {
    const oldi = i;
    let w = 1;
    let k = PUNYCODE_BASE;
    while (true) {
      if (inIdx >= value.length)
        throw new Error("Invalid punycode: unexpected end of input");
      const ch = value.charCodeAt(inIdx++);
      let digit;
      if (ch >= 97 && ch <= 122)
        digit = ch - 97;
      else if (ch >= 48 && ch <= 57)
        digit = ch - 48 + 26;
      else if (ch >= 65 && ch <= 90)
        digit = ch - 65;
      else
        throw new Error("Invalid punycode: bad digit character");
      i += digit * w;
      const t = k <= bias ? PUNYCODE_TMIN : k >= bias + PUNYCODE_TMAX ? PUNYCODE_TMAX : k - bias;
      if (digit < t)
        break;
      w *= PUNYCODE_BASE - t;
      k += PUNYCODE_BASE;
    }
    const outLen = output.length + 1;
    bias = Adapt(i - oldi, outLen, oldi === 0);
    n += Math.floor(i / outLen);
    i %= outLen;
    output.splice(i, 0, n);
    i++;
  }
  return globalThis.String.fromCodePoint(...output);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/_idna.mjs
function IsNonspacingMark(cp) {
  return /\p{Mn}/u.test(String.fromCodePoint(cp));
}
function IsSpacingCombiningMark(cp) {
  return /\p{Mc}/u.test(String.fromCodePoint(cp));
}
function IsEnclosingMark(cp) {
  return /\p{Me}/u.test(String.fromCodePoint(cp));
}
function IsCombiningMark2(cp) {
  return IsNonspacingMark(cp) || IsSpacingCombiningMark(cp) || IsEnclosingMark(cp);
}
var RFC5892_DISALLOWED = new Set([
  1600,
  2042,
  12334,
  12335,
  12337,
  12338,
  12339,
  12340,
  12341,
  12347
]);
var VIRAMA_CPS = new Set([
  2381,
  2509,
  2637,
  2765,
  2893,
  3021,
  3149,
  3277,
  3387,
  3388,
  3405,
  3530,
  6980,
  7082,
  7083,
  43456,
  69702,
  69759,
  69817,
  69939,
  69940,
  70080,
  70197,
  70477,
  70722,
  70850,
  71103,
  71231,
  71350,
  72767,
  73028,
  73029
]);
function IsGreek(cp) {
  return /\p{Script=Greek}/u.test(String.fromCodePoint(cp));
}
function IsHebrew(cp) {
  return /\p{Script=Hebrew}/u.test(String.fromCodePoint(cp));
}
function IsHiragana(cp) {
  return /\p{Script=Hiragana}/u.test(String.fromCodePoint(cp));
}
function IsKatakana(cp) {
  return /\p{Script=Katakana}/u.test(String.fromCodePoint(cp));
}
function IsHan(cp) {
  return /\p{Script=Han}/u.test(String.fromCodePoint(cp));
}
function IsArabicIndicDigit(cp) {
  return cp >= 1632 && cp <= 1641;
}
function IsExtendedArabicIndicDigit(cp) {
  return cp >= 1776 && cp <= 1785;
}
function IsVirama(cp) {
  return VIRAMA_CPS.has(cp);
}
function IsUnicodeLabel(value) {
  if (value.length === 0)
    return false;
  const cps = [...value].map((c) => c.codePointAt(0));
  const len = cps.length;
  if (cps[0] === 45 || cps[len - 1] === 45)
    return false;
  if (len >= 4 && cps[2] === 45 && cps[3] === 45)
    return false;
  if (IsCombiningMark2(cps[0]))
    return false;
  let hasJapanese = false;
  let hasArabicIndic = false;
  let hasExtendedArabicIndic = false;
  for (let i = 0;i < len; i++) {
    const cp = cps[i];
    if (RFC5892_DISALLOWED.has(cp))
      return false;
    if (IsHiragana(cp) || IsKatakana(cp) || IsHan(cp))
      hasJapanese = true;
    if (IsArabicIndicDigit(cp))
      hasArabicIndic = true;
    if (IsExtendedArabicIndicDigit(cp))
      hasExtendedArabicIndic = true;
    const prev = cps[i - 1], next = cps[i + 1];
    switch (cp) {
      case 183:
        if (prev !== 108 || next !== 108)
          return false;
        break;
      case 885:
        if (next === undefined || !IsGreek(next))
          return false;
        break;
      case 1523:
      case 1524:
        if (prev === undefined || !IsHebrew(prev))
          return false;
        break;
      case 8205:
        if (prev === undefined || !IsVirama(prev))
          return false;
        break;
      case 12539:
        break;
    }
  }
  if (value.includes("・") && !hasJapanese)
    return false;
  if (hasArabicIndic && hasExtendedArabicIndic)
    return false;
  return true;
}
function IsAsciiLabel(value) {
  if (value.charCodeAt(0) === 45 || value.charCodeAt(value.length - 1) === 45)
    return false;
  if (value.length >= 4 && value.charCodeAt(2) === 45 && value.charCodeAt(3) === 45)
    return false;
  for (let i = 0;i < value.length; i++) {
    const ch = value.charCodeAt(i);
    if (!(ch >= 97 && ch <= 122 || ch >= 65 && ch <= 90 || ch >= 48 && ch <= 57 || ch === 45))
      return false;
  }
  return true;
}
function IsPuny(value) {
  return value.toLowerCase().startsWith("xn--");
}
function IsPunyLabel(value) {
  try {
    return IsUnicodeLabel(Decode(value.slice(4)));
  } catch {
    return false;
  }
}
function IsIdnLabel(value) {
  if (value.length === 0 || value.length > 63)
    return false;
  return IsPuny(value) ? IsPunyLabel(value) : IsUnicodeLabel(value);
}
function IsLabel(value) {
  if (value.length === 0 || value.length > 63)
    return false;
  return IsPuny(value) ? IsPunyLabel(value) : IsAsciiLabel(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/hostname.mjs
function IsHostname(value) {
  if (value.length === 0 || value.length > 253)
    return false;
  if (value.charCodeAt(value.length - 1) === 46)
    return false;
  for (const label of value.split(".")) {
    if (!IsLabel(label))
      return false;
  }
  return true;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/idn_email.mjs
var IdnEmail = /^(?!.*\.\.)[\p{L}\p{N}!#$%&'*+/=?^_`{|}~-]+(?:\.[\p{L}\p{N}!#$%&'*+/=?^_`{|}~-]+)*@[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,61}[\p{L}\p{N}])?(?:\.[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,61}[\p{L}\p{N}])?)*$/iu;
function IsIdnEmail(value) {
  return IdnEmail.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/idn_hostname.mjs
function IsIdnHostname(value) {
  if (value.length === 0 || value.includes(" "))
    return false;
  const canonical = value.normalize("NFC").replace(/[\u002E\u3002\uFF0E\uFF61]/g, ".");
  if (canonical.length > 253)
    return false;
  for (const label of canonical.split(".")) {
    if (!IsIdnLabel(label))
      return false;
  }
  return true;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/ipv4.mjs
function IsIPv4Internal(value, start, end) {
  let dots = 0;
  let num = 0;
  let digits = 0;
  let leading = 0;
  for (let i = start;i < end; i++) {
    const ch = value.charCodeAt(i);
    if (ch === 46) {
      if (digits === 0 || num > 255 || leading === 48 && digits > 1)
        return false;
      dots++;
      num = 0;
      digits = 0;
      leading = 0;
    } else if (ch >= 48 && ch <= 57) {
      if (digits === 0)
        leading = ch;
      num = num * 10 + (ch - 48);
      digits++;
    } else {
      return false;
    }
  }
  return dots === 3 && digits > 0 && num <= 255 && !(leading === 48 && digits > 1);
}
function IsIPv4(value) {
  return IsIPv4Internal(value, 0, value.length);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/ipv6.mjs
function InRange(ch) {
  return ch >= 48 && ch <= 57 || ch >= 65 && ch <= 70 || ch >= 97 && ch <= 102;
}
function IsIPv6(value) {
  const length = value.length;
  if (length === 0)
    return false;
  let groups = 0;
  let compressed = false;
  let i = 0;
  if (value.charCodeAt(0) === 58 && value.charCodeAt(1) === 58) {
    if (length === 2)
      return true;
    compressed = true;
    i = 2;
  }
  while (i < length) {
    let digits = 0;
    const start = i;
    while (i < length && InRange(value.charCodeAt(i))) {
      i++;
      digits++;
    }
    if (digits === 0)
      return false;
    const next = value.charCodeAt(i);
    if (next === 46) {
      if (!IsIPv4Internal(value, start, length))
        return false;
      groups += 2;
      i = length;
      break;
    }
    if (digits > 4)
      return false;
    groups++;
    if (i === length)
      break;
    if (next !== 58)
      return false;
    i++;
    if (value.charCodeAt(i) === 58) {
      if (compressed)
        return false;
      if (value.charCodeAt(i + 1) === 58)
        return false;
      compressed = true;
      i++;
      if (i === length)
        break;
    }
  }
  return compressed ? groups <= 7 : groups === 8;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/iri_reference.mjs
function TryUrl(value) {
  try {
    new URL(value, "http://example.com");
    return true;
  } catch {
    return false;
  }
}
function IsIriReference(value) {
  if (value.includes(" ")) {
    return false;
  }
  if (value.includes("\\")) {
    return false;
  }
  if (/[\x00-\x1F\x7F]/.test(value)) {
    return false;
  }
  if (/%(?![0-9a-fA-F]{2})/.test(value)) {
    return false;
  }
  if (value === "") {
    return true;
  }
  const colonIndex = value.indexOf(":");
  const hasValidSchemePrefix = colonIndex > 0 && /^[a-zA-Z][a-zA-Z0-9+\-.]*$/.test(value.substring(0, colonIndex));
  if (hasValidSchemePrefix) {
    return TryUrl(value);
  } else {
    const looksLikeMalformedSchemeAndAuthority = value.match(/^([a-zA-Z][a-zA-Z0-9+\-.]*)(\/\/)/);
    if (looksLikeMalformedSchemeAndAuthority && colonIndex === -1) {
      return false;
    }
    return TryUrl(value);
  }
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/iri.mjs
function IsIri(value) {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/json_pointer_uri_fragment.mjs
var JsonPointerUriFragment = /^#(?:\/(?:[a-z0-9_\-.!$&'()*+,;:=@]|%[0-9a-f]{2}|~0|~1)*)*$/i;
function IsJsonPointerUriFragment(value) {
  return JsonPointerUriFragment.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/json_pointer.mjs
var JsonPointer = /^(?:\/(?:[^~/]|~0|~1)*)*$/;
function IsJsonPointer(value) {
  return JsonPointer.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/regex.mjs
function IsRegex(value) {
  if (value.length === 0) {
    return false;
  }
  try {
    new RegExp(value);
    return true;
  } catch {
    return false;
  }
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/relative_json_pointer.mjs
var RelativeJsonPointer = /^(?:0|[1-9][0-9]*)(?:#|(?:\/(?:[^~/]|~0|~1)*)*)$/;
function IsRelativeJsonPointer(value) {
  return RelativeJsonPointer.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/uri_reference.mjs
var UriReference = /^(?!.*[^\x00-\x7F])(?!.*\\)(?:(?:[a-z][a-z0-9+\-.]*:)?(?:\/\/[^\s[\]{}<>^`|]*)?|[^\s[\]{}<>^`|]*)(?:\?[^\s[\]{}<>^`|]*)?(?:#[^\s[\]{}<>^`|]*)?$/i;
function IsUriReference(value) {
  return UriReference.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/uri_template.mjs
var UriTemplate = /^(?:(?:[^\x00-\x20"'<>%\\^`{|}]|%[0-9a-f]{2})|\{[+#./;?&=,!@|]?(?:[a-z0-9_]|%[0-9a-f]{2})+(?::[1-9][0-9]{0,3}|\*)?(?:,(?:[a-z0-9_]|%[0-9a-f]{2})+(?::[1-9][0-9]{0,3}|\*)?)*\})*$/i;
function IsUriTemplate(value) {
  return UriTemplate.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/uri.mjs
function IsAlpha(ch) {
  return ch >= 97 && ch <= 122 || ch >= 65 && ch <= 90;
}
function IsAlphaNumeric(ch) {
  return IsAlpha(ch) || ch >= 48 && ch <= 57;
}
function IsHex(ch) {
  return ch >= 48 && ch <= 57 || ch >= 65 && ch <= 70 || ch >= 97 && ch <= 102;
}
function IsSchemeChar(ch) {
  return IsAlphaNumeric(ch) || ch === 43 || ch === 45 || ch === 46;
}
function IsUnreserved(ch) {
  return IsAlphaNumeric(ch) || ch === 45 || ch === 46 || ch === 95 || ch === 126;
}
function IsSubDelim(ch) {
  return ch === 33 || ch === 36 || ch === 38 || ch === 39 || ch === 40 || ch === 41 || ch === 42 || ch === 43 || ch === 44 || ch === 59 || ch === 61;
}
function IsPchar(ch) {
  return IsUnreserved(ch) || IsSubDelim(ch) || ch === 58 || ch === 64;
}
function IsUri(value) {
  const length = value.length;
  if (length === 0)
    return false;
  if (!IsAlpha(value.charCodeAt(0)))
    return false;
  let i = 1;
  while (i < length) {
    const ch = value.charCodeAt(i);
    if (ch === 58)
      break;
    if (!IsSchemeChar(ch))
      return false;
    i++;
  }
  if (value.charCodeAt(i) !== 58)
    return false;
  i++;
  if (value.charCodeAt(i) === 47 && value.charCodeAt(i + 1) === 47) {
    i += 2;
    const authorityStart = i;
    let atPos = -1;
    for (let j = i;j < length; j++) {
      const ch = value.charCodeAt(j);
      if (ch === 64) {
        atPos = j;
        break;
      }
      if (ch === 47 || ch === 63 || ch === 35)
        break;
    }
    if (atPos !== -1) {
      for (let j = authorityStart;j < atPos; j++) {
        const ch = value.charCodeAt(j);
        if (ch === 91 || ch === 93)
          return false;
        if (ch === 37) {
          if (j + 2 >= atPos || !IsHex(value.charCodeAt(j + 1)) || !IsHex(value.charCodeAt(j + 2)))
            return false;
          j += 2;
        } else if (!IsUnreserved(ch) && !IsSubDelim(ch) && ch !== 58)
          return false;
      }
      i = atPos + 1;
    }
    if (value.charCodeAt(i) === 91) {
      i++;
      while (i < length && value.charCodeAt(i) !== 93)
        i++;
      if (value.charCodeAt(i) !== 93)
        return false;
      i++;
    } else {
      while (i < length) {
        const ch = value.charCodeAt(i);
        if (ch === 47 || ch === 63 || ch === 35 || ch === 58)
          break;
        if (ch < 128 && !IsUnreserved(ch) && !IsSubDelim(ch))
          return false;
        i++;
      }
    }
    if (value.charCodeAt(i) === 58) {
      i++;
      while (i < length) {
        const ch = value.charCodeAt(i);
        if (ch === 47 || ch === 63 || ch === 35)
          break;
        if (ch < 48 || ch > 57)
          return false;
        i++;
      }
    }
  }
  while (i < length) {
    const ch = value.charCodeAt(i);
    if (ch === 37) {
      if (i + 2 >= length || !IsHex(value.charCodeAt(i + 1)) || !IsHex(value.charCodeAt(i + 2)))
        return false;
      i += 2;
    } else if (ch > 127) {
      return false;
    } else if (!(IsPchar(ch) || ch === 47 || ch === 63 || ch === 35)) {
      return false;
    }
    i++;
  }
  return true;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/url.mjs
var Url = /^(?:https?|ftp):\/\/(?:\S+(?::\S*)?@)?(?:(?!(?:10|127)(?:\.\d{1,3}){3})(?!(?:169\.254|192\.168)(?:\.\d{1,3}){2})(?!172\.(?:1[6-9]|2\d|3[0-1])(?:\.\d{1,3}){2})(?:[1-9]\d?|1\d\d|2[01]\d|22[0-3])(?:\.(?:1?\d{1,2}|2[0-4]\d|25[0-5])){2}(?:\.(?:[1-9]\d?|1\d\d|2[0-4]\d|25[0-4]))|(?:(?:[a-z0-9\u{00a1}-\u{ffff}]+-)*[a-z0-9\u{00a1}-\u{ffff}]+)(?:\.(?:[a-z0-9\u{00a1}-\u{ffff}]+-)*[a-z0-9\u{00a1}-\u{ffff}]+)*(?:\.(?:[a-z\u{00a1}-\u{ffff}]{2,})))(?::\d{2,5})?(?:\/[^\s]*)?$/iu;
function IsUrl(value) {
  return Url.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/uuid.mjs
var Uuid = /^(?:urn:uuid:)?[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i;
function IsUuid(value) {
  return Uuid.test(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/format/_registry.mjs
var formats = new Map;
function Clear() {
  formats.clear();
}
function Entries3() {
  return [...formats.entries()];
}
function Set2(format2, check) {
  formats.set(format2, check);
}
function Has(format2) {
  return formats.has(format2);
}
function Get(format2) {
  return formats.get(format2);
}
function Test(format2, value) {
  return formats.get(format2)?.(value) ?? true;
}
function Reset() {
  Clear();
  formats.set("date-time", IsDateTime);
  formats.set("date", IsDate2);
  formats.set("duration", IsDuration);
  formats.set("email", IsEmail);
  formats.set("hostname", IsHostname);
  formats.set("idn-email", IsIdnEmail);
  formats.set("idn-hostname", IsIdnHostname);
  formats.set("ipv4", IsIPv4);
  formats.set("ipv6", IsIPv6);
  formats.set("iri-reference", IsIriReference);
  formats.set("iri", IsIri);
  formats.set("json-pointer-uri-fragment", IsJsonPointerUriFragment);
  formats.set("json-pointer", IsJsonPointer);
  formats.set("regex", IsRegex);
  formats.set("relative-json-pointer", IsRelativeJsonPointer);
  formats.set("time", IsTime);
  formats.set("uri-reference", IsUriReference);
  formats.set("uri-template", IsUriTemplate);
  formats.set("uri", IsUri);
  formats.set("url", IsUrl);
  formats.set("uuid", IsUuid);
}
Reset();
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/format.mjs
function BuildFormat(_stack, _context, schema2, value) {
  return exports_emit.Call(exports_emit.Member("Format", "Test"), [exports_emit.Constant(schema2.format), value]);
}
function CheckFormat(_stack, _context, schema2, value) {
  return exports_format.Test(schema2.format, value);
}
function ErrorFormat(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckFormat(stack, context, schema2, value) || context.AddError({
    keyword: "format",
    schemaPath,
    instancePath,
    params: { format: schema2.format }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/if.mjs
function BuildIf(stack, context, schema2, value) {
  const thenSchema = IsThen(schema2) ? schema2.then : true;
  const elseSchema = IsElse(schema2) ? schema2.else : true;
  return exports_emit.Ternary(BuildSchema(stack, context, schema2.if, value), BuildSchema(stack, context, thenSchema, value), BuildSchema(stack, context, elseSchema, value));
}
function CheckIf(stack, context, schema2, value) {
  const thenSchema = IsThen(schema2) ? schema2.then : true;
  const elseSchema = IsElse(schema2) ? schema2.else : true;
  return CheckSchema(stack, context, schema2.if, value) ? CheckSchema(stack, context, thenSchema, value) : CheckSchema(stack, context, elseSchema, value);
}
function ErrorIf(stack, context, schemaPath, instancePath, schema2, value) {
  const thenSchema = IsThen(schema2) ? schema2.then : true;
  const elseSchema = IsElse(schema2) ? schema2.else : true;
  const trueContext = new AccumulatedErrorContext;
  const isIf = ErrorSchema(stack, trueContext, `${schemaPath}/if`, instancePath, schema2.if, value) ? ErrorSchema(stack, trueContext, `${schemaPath}/then`, instancePath, thenSchema, value) || context.AddError({
    keyword: "if",
    schemaPath,
    instancePath,
    params: { failingKeyword: "then" }
  }) : ErrorSchema(stack, context, `${schemaPath}/else`, instancePath, elseSchema, value) || context.AddError({
    keyword: "if",
    schemaPath,
    instancePath,
    params: { failingKeyword: "else" }
  });
  if (isIf)
    context.Merge([trueContext]);
  return isIf;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/items.mjs
function BuildItemsSized(stack, context, schema2, value) {
  return exports_emit.ReduceAnd(schema2.items.map((schema3, index2) => {
    const isLength = exports_emit.IsLessEqualThan(exports_emit.Member(value, "length"), exports_emit.Constant(index2));
    const isSchema = BuildSchemaPushStack(stack, context, schema3, `${value}[${index2}]`);
    const addIndex = context.AddIndex(exports_emit.Constant(index2));
    const guarded = context.UseUnevaluated() ? exports_emit.And(isSchema, addIndex) : isSchema;
    return exports_emit.Or(isLength, guarded);
  }));
}
function CheckItemsSized(stack, context, schema2, value) {
  return exports_guard.Every(schema2.items, 0, (schema3, index2) => {
    return exports_guard.IsLessEqualThan(value.length, index2) || CheckSchemaPushStack(stack, context, schema3, value[index2]) && context.AddIndex(index2);
  });
}
function ErrorItemsSized(stack, context, schemaPath, instancePath, schema2, value) {
  return exports_guard.EveryAll(schema2.items, 0, (schema3, index2) => {
    const nextSchemaPath = `${schemaPath}/items/${index2}`;
    const nextInstancePath = `${instancePath}/${index2}`;
    return exports_guard.IsLessEqualThan(value.length, index2) || ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema3, value[index2]) && context.AddIndex(index2);
  });
}
function BuildItemsUnsized(stack, context, schema2, value) {
  const offset = IsPrefixItems(schema2) ? schema2.prefixItems.length : 0;
  const isSchema = BuildSchemaPushStack(stack, context, schema2.items, "element");
  const addIndex = context.AddIndex("index");
  const guarded = context.UseUnevaluated() ? exports_emit.And(isSchema, addIndex) : isSchema;
  return exports_emit.Every(value, exports_emit.Constant(offset), ["element", "index"], guarded);
}
function CheckItemsUnsized(stack, context, schema2, value) {
  const offset = IsPrefixItems(schema2) ? schema2.prefixItems.length : 0;
  return exports_guard.Every(value, offset, (element, index2) => {
    return CheckSchemaPushStack(stack, context, schema2.items, element) && context.AddIndex(index2);
  });
}
function ErrorItemsUnsized(stack, context, schemaPath, instancePath, schema2, value) {
  const offset = IsPrefixItems(schema2) ? schema2.prefixItems.length : 0;
  return exports_guard.EveryAll(value, offset, (element, index2) => {
    const nextSchemaPath = `${schemaPath}/items`;
    const nextInstancePath = `${instancePath}/${index2}`;
    return ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema2.items, element) && context.AddIndex(index2);
  });
}
function BuildItems(stack, context, schema2, value) {
  return IsItemsSized(schema2) ? BuildItemsSized(stack, context, schema2, value) : BuildItemsUnsized(stack, context, schema2, value);
}
function CheckItems(stack, context, schema2, value) {
  return IsItemsSized(schema2) ? CheckItemsSized(stack, context, schema2, value) : CheckItemsUnsized(stack, context, schema2, value);
}
function ErrorItems(stack, context, schemaPath, instancePath, schema2, value) {
  return IsItemsSized(schema2) ? ErrorItemsSized(stack, context, schemaPath, instancePath, schema2, value) : ErrorItemsUnsized(stack, context, schemaPath, instancePath, schema2, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/maxContains.mjs
function IsValid3(schema2) {
  return IsContains(schema2);
}
function BuildMaxContains(stack, context, schema2, value) {
  if (!IsValid3(schema2))
    return exports_emit.Constant(true);
  const [result, item] = [Unique(), Unique()];
  const count = exports_emit.Call(exports_emit.Member(value, "reduce"), [exports_emit.ArrowFunction([result, item], exports_emit.Ternary(BuildSchema(stack, context, schema2.contains, item), exports_emit.PrefixIncrement(result), result)), exports_emit.Constant(0)]);
  return exports_emit.IsLessEqualThan(count, exports_emit.Constant(schema2.maxContains));
}
function CheckMaxContains(stack, context, schema2, value) {
  if (!IsValid3(schema2))
    return true;
  const count = value.reduce((result, item) => CheckSchema(stack, context, schema2.contains, item) ? ++result : result, 0);
  return exports_guard.IsLessEqualThan(count, schema2.maxContains);
}
function ErrorMaxContains(stack, context, schemaPath, instancePath, schema2, value) {
  const minContains2 = IsMinContains(schema2) ? schema2.minContains : 1;
  return CheckMaxContains(stack, context, schema2, value) || context.AddError({
    keyword: "contains",
    schemaPath,
    instancePath,
    params: { minContains: minContains2, maxContains: schema2.maxContains }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/maximum.mjs
function BuildMaximum(_stack, _context, schema2, value) {
  return exports_emit.IsLessEqualThan(value, exports_emit.Constant(schema2.maximum));
}
function CheckMaximum(_stack, _context, schema2, value) {
  return exports_guard.IsLessEqualThan(value, schema2.maximum);
}
function ErrorMaximum(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMaximum(stack, context, schema2, value) || context.AddError({
    keyword: "maximum",
    schemaPath,
    instancePath,
    params: { comparison: "<=", limit: schema2.maximum }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/maxItems.mjs
function BuildMaxItems(_stack, _context, schema2, value) {
  return exports_emit.IsLessEqualThan(exports_emit.Member(value, "length"), exports_emit.Constant(schema2.maxItems));
}
function CheckMaxItems(_stack, _context, schema2, value) {
  return exports_guard.IsLessEqualThan(value.length, schema2.maxItems);
}
function ErrorMaxItems(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMaxItems(stack, context, schema2, value) || context.AddError({
    keyword: "maxItems",
    schemaPath,
    instancePath,
    params: { limit: schema2.maxItems }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/maxLength.mjs
function BuildMaxLength(_stack, _context, schema2, value) {
  return exports_emit.IsMaxLength(value, exports_emit.Constant(schema2.maxLength));
}
function CheckMaxLength(_stack, _context, schema2, value) {
  return exports_guard.IsMaxLength(value, schema2.maxLength);
}
function ErrorMaxLength(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMaxLength(stack, context, schema2, value) || context.AddError({
    keyword: "maxLength",
    schemaPath,
    instancePath,
    params: { limit: schema2.maxLength }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/maxProperties.mjs
function BuildMaxProperties(_stack, _context, schema2, value) {
  return exports_emit.IsLessEqualThan(exports_emit.Member(exports_emit.Keys(value), "length"), exports_emit.Constant(schema2.maxProperties));
}
function CheckMaxProperties(_stack, _context, schema2, value) {
  return exports_guard.IsLessEqualThan(exports_guard.Keys(value).length, schema2.maxProperties);
}
function ErrorMaxProperties(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMaxProperties(stack, context, schema2, value) || context.AddError({
    keyword: "maxProperties",
    schemaPath,
    instancePath,
    params: { limit: schema2.maxProperties }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/minContains.mjs
function IsValid4(schema2) {
  return IsContains(schema2);
}
function BuildMinContains(stack, context, schema2, value) {
  if (!IsValid4(schema2))
    return exports_emit.Constant(true);
  const [result, item] = [Unique(), Unique()];
  const count = exports_emit.Call(exports_emit.Member(value, "reduce"), [exports_emit.ArrowFunction([result, item], exports_emit.Ternary(BuildSchema(stack, context, schema2.contains, item), exports_emit.PrefixIncrement(result), result)), exports_emit.Constant(0)]);
  return exports_emit.IsGreaterEqualThan(count, exports_emit.Constant(schema2.minContains));
}
function CheckMinContains(stack, context, schema2, value) {
  if (!IsValid4(schema2))
    return true;
  const count = value.reduce((result, item) => CheckSchema(stack, context, schema2.contains, item) ? ++result : result, 0);
  return exports_guard.IsGreaterEqualThan(count, schema2.minContains);
}
function ErrorMinContains(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMinContains(stack, context, schema2, value) || context.AddError({
    keyword: "contains",
    schemaPath,
    instancePath,
    params: { minContains: schema2.minContains }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/minimum.mjs
function BuildMinimum(_stack, _context, schema2, value) {
  return exports_emit.IsGreaterEqualThan(value, exports_emit.Constant(schema2.minimum));
}
function CheckMinimum(_stack, _context, schema2, value) {
  return exports_guard.IsGreaterEqualThan(value, schema2.minimum);
}
function ErrorMinimum(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMinimum(stack, context, schema2, value) || context.AddError({
    keyword: "minimum",
    schemaPath,
    instancePath,
    params: { comparison: ">=", limit: schema2.minimum }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/minItems.mjs
function BuildMinItems(_stack, _context, schema2, value) {
  return exports_emit.IsGreaterEqualThan(exports_emit.Member(value, "length"), exports_emit.Constant(schema2.minItems));
}
function CheckMinItems(_stack, _context, schema2, value) {
  return exports_guard.IsGreaterEqualThan(value.length, schema2.minItems);
}
function ErrorMinItems(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMinItems(stack, context, schema2, value) || context.AddError({
    keyword: "minItems",
    schemaPath,
    instancePath,
    params: { limit: schema2.minItems }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/minLength.mjs
function BuildMinLength(_stack, _context, schema2, value) {
  return exports_emit.IsMinLength(value, exports_emit.Constant(schema2.minLength));
}
function CheckMinLength(_stack, _context, schema2, value) {
  return exports_guard.IsMinLength(value, schema2.minLength);
}
function ErrorMinLength(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMinLength(stack, context, schema2, value) || context.AddError({
    keyword: "minLength",
    schemaPath,
    instancePath,
    params: { limit: schema2.minLength }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/minProperties.mjs
function BuildMinProperties(_stack, _context, schema2, value) {
  return exports_emit.IsGreaterEqualThan(exports_emit.Member(exports_emit.Keys(value), "length"), exports_emit.Constant(schema2.minProperties));
}
function CheckMinProperties(_stack, _context, schema2, value) {
  return exports_guard.IsGreaterEqualThan(exports_guard.Keys(value).length, schema2.minProperties);
}
function ErrorMinProperties(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMinProperties(stack, context, schema2, value) || context.AddError({
    keyword: "minProperties",
    schemaPath,
    instancePath,
    params: { limit: schema2.minProperties }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/multipleOf.mjs
function BuildMultipleOf(_stack, _context, schema2, value) {
  return exports_emit.MultipleOf(value, exports_emit.Constant(schema2.multipleOf));
}
function CheckMultipleOf(_stack, _context, schema2, value) {
  return exports_guard.IsMultipleOf(value, schema2.multipleOf);
}
function ErrorMultipleOf(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckMultipleOf(stack, context, schema2, value) || context.AddError({
    keyword: "multipleOf",
    schemaPath,
    instancePath,
    params: { multipleOf: schema2.multipleOf }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/not.mjs
function BuildNotUnevaluated(stack, context, schema2, value) {
  return Reducer(stack, context, [schema2.not], value, exports_emit.Not(exports_emit.IsEqual(exports_emit.Member("results", "length"), exports_emit.Constant(1))));
}
function BuildNotFast(stack, context, schema2, value) {
  return exports_emit.Not(BuildSchema(stack, context, schema2.not, value));
}
function BuildNot(stack, context, schema2, value) {
  return context.UseUnevaluated() ? BuildNotUnevaluated(stack, context, schema2, value) : BuildNotFast(stack, context, schema2, value);
}
function CheckNot(stack, context, schema2, value) {
  const nextContext = new CheckContext;
  const isSchema = !CheckSchema(stack, nextContext, schema2.not, value);
  const isNot = isSchema && context.Merge([nextContext]);
  return isNot;
}
function ErrorNot(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckNot(stack, context, schema2, value) || context.AddError({
    keyword: "not",
    schemaPath,
    instancePath,
    params: {}
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/oneOf.mjs
function BuildOneOfUnevaluated(stack, context, schema2, value) {
  return Reducer(stack, context, schema2.oneOf, value, exports_emit.IsEqual(exports_emit.Member("results", "length"), exports_emit.Constant(1)));
}
function BuildOneOfFast(stack, context, schema2, value) {
  const results = exports_emit.ArrayLiteral(schema2.oneOf.map((schema3) => BuildSchema(stack, context, schema3, value)));
  const count = exports_emit.Call(exports_emit.Member(results, "reduce"), [
    exports_emit.ArrowFunction(["count", "result"], exports_emit.Ternary(exports_emit.IsEqual("result", exports_emit.Constant(true)), exports_emit.PrefixIncrement("count"), "count")),
    exports_emit.Constant(0)
  ]);
  return exports_emit.IsEqual(count, exports_emit.Constant(1));
}
function BuildOneOf(stack, context, schema2, value) {
  return context.UseUnevaluated() ? BuildOneOfUnevaluated(stack, context, schema2, value) : BuildOneOfFast(stack, context, schema2, value);
}
function CheckOneOf(stack, context, schema2, value) {
  const passedContexts = schema2.oneOf.reduce((result, schema3) => {
    const nextContext = new CheckContext;
    return CheckSchema(stack, nextContext, schema3, value) ? [...result, nextContext] : result;
  }, []);
  return exports_guard.IsEqual(passedContexts.length, 1) && context.Merge(passedContexts);
}
function ErrorOneOf(stack, context, schemaPath, instancePath, schema2, value) {
  const failedContexts = [];
  const passingSchemas = [];
  const passedContexts = schema2.oneOf.reduce((result, schema3, index2) => {
    const nextContext = new AccumulatedErrorContext;
    const nextSchemaPath = `${schemaPath}/oneOf/${index2}`;
    const isSchema = ErrorSchema(stack, nextContext, nextSchemaPath, instancePath, schema3, value);
    if (isSchema)
      passingSchemas.push(index2);
    if (!isSchema)
      failedContexts.push(nextContext);
    return isSchema ? [...result, nextContext] : result;
  }, []);
  const isOneOf = exports_guard.IsEqual(passedContexts.length, 1) && context.Merge(passedContexts);
  if (!isOneOf && exports_guard.IsEqual(passingSchemas.length, 0))
    failedContexts.forEach((failed) => failed.GetErrors().forEach((error) => context.AddError(error)));
  return isOneOf || context.AddError({
    keyword: "oneOf",
    schemaPath,
    instancePath,
    params: { passingSchemas }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/pattern.mjs
function BuildPattern(_stack, _context, schema2, value) {
  const regexp = CreateVariable(exports_guard.IsString(schema2.pattern) ? new RegExp(schema2.pattern, "u") : schema2.pattern);
  return exports_emit.Call(exports_emit.Member(regexp, "test"), [value]);
}
function CheckPattern(_stack, _context, schema2, value) {
  const regexp = exports_guard.IsString(schema2.pattern) ? new RegExp(schema2.pattern, "u") : schema2.pattern;
  return regexp.test(value);
}
function ErrorPattern(stack, context, schemaPath, instancePath, schema2, value) {
  return CheckPattern(stack, context, schema2, value) || context.AddError({
    keyword: "pattern",
    schemaPath,
    instancePath,
    params: { pattern: schema2.pattern }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/patternProperties.mjs
function BuildPatternProperties(stack, context, schema2, value) {
  return exports_emit.ReduceAnd(exports_guard.Entries(schema2.patternProperties).map(([pattern2, schema3]) => {
    const [key, prop] = [Unique(), Unique()];
    const regexp = CreateVariable(new RegExp(pattern2, "u"));
    const notKey = exports_emit.Not(exports_emit.Call(exports_emit.Member(regexp, "test"), [key]));
    const isSchema = BuildSchemaPushStack(stack, context, schema3, prop);
    const addKey = context.AddKey(key);
    const guarded = context.UseUnevaluated() ? exports_emit.Or(notKey, exports_emit.And(isSchema, addKey)) : exports_emit.Or(notKey, isSchema);
    return exports_emit.Every(exports_emit.Entries(value), exports_emit.Constant(0), [`[${key}, ${prop}]`, "_"], guarded);
  }));
}
function CheckPatternProperties(stack, context, schema2, value) {
  return exports_guard.Every(exports_guard.Entries(schema2.patternProperties), 0, ([pattern2, schema3]) => {
    const regexp = new RegExp(pattern2, "u");
    return exports_guard.Every(exports_guard.Entries(value), 0, ([key, prop]) => {
      return !regexp.test(key) || CheckSchemaPushStack(stack, context, schema3, prop) && context.AddKey(key);
    });
  });
}
function ErrorPatternProperties(stack, context, schemaPath, instancePath, schema2, value) {
  return exports_guard.EveryAll(exports_guard.Entries(schema2.patternProperties), 0, ([pattern2, schema3]) => {
    const nextSchemaPath = `${schemaPath}/patternProperties/${pattern2}`;
    const regexp = new RegExp(pattern2, "u");
    return exports_guard.EveryAll(exports_guard.Entries(value), 0, ([key, value2]) => {
      const nextInstancePath = `${instancePath}/${key}`;
      const notKey = !regexp.test(key);
      return notKey || ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema3, value2) && context.AddKey(key);
    });
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/prefixItems.mjs
function BuildPrefixItems(stack, context, schema2, value) {
  return exports_emit.ReduceAnd(schema2.prefixItems.map((schema3, index2) => {
    const isLength = exports_emit.IsLessEqualThan(exports_emit.Member(value, "length"), exports_emit.Constant(index2));
    const isSchema = BuildSchemaPushStack(stack, context, schema3, `${value}[${index2}]`);
    const addIndex = context.AddIndex(exports_emit.Constant(index2));
    const guarded = context.UseUnevaluated() ? exports_emit.And(isSchema, addIndex) : isSchema;
    return exports_emit.Or(isLength, guarded);
  }));
}
function CheckPrefixItems(stack, context, schema2, value) {
  return exports_guard.IsEqual(value.length, 0) || exports_guard.Every(schema2.prefixItems, 0, (schema3, index2) => {
    return exports_guard.IsLessEqualThan(value.length, index2) || CheckSchemaPushStack(stack, context, schema3, value[index2]) && context.AddIndex(index2);
  });
}
function ErrorPrefixItems(stack, context, schemaPath, instancePath, schema2, value) {
  return exports_guard.IsEqual(value.length, 0) || exports_guard.EveryAll(schema2.prefixItems, 0, (schema3, index2) => {
    const nextSchemaPath = `${schemaPath}/prefixItems/${index2}`;
    const nextInstancePath = `${instancePath}/${index2}`;
    return exports_guard.IsLessEqualThan(value.length, index2) || ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema3, value[index2]) && context.AddIndex(index2);
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/settings/settings.mjs
var exports_settings = {};
__export(exports_settings, {
  Set: () => Set3,
  Reset: () => Reset2,
  Get: () => Get2
});
var settings = {
  immutableTypes: false,
  maxErrors: 8,
  useAcceleration: true,
  exactOptionalPropertyTypes: false,
  enumerableKind: false,
  correctiveParse: false
};
function Reset2() {
  settings.immutableTypes = false;
  settings.maxErrors = 8;
  settings.useAcceleration = true;
  settings.exactOptionalPropertyTypes = false;
  settings.enumerableKind = false;
  settings.correctiveParse = false;
}
function Set3(options) {
  for (const key of exports_guard.Keys(options)) {
    const value = options[key];
    if (value !== undefined) {
      Object.defineProperty(settings, key, { value });
    }
  }
}
function Get2() {
  return settings;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_exact_optional.mjs
function IsExactOptional(required2, key) {
  return required2.includes(key) || exports_settings.Get().exactOptionalPropertyTypes;
}
function InexactOptionalBuild(value, key) {
  return exports_emit.IsUndefined(exports_emit.Member(value, key));
}
function InexactOptionalCheck(value, key) {
  return exports_guard.IsUndefined(value[key]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/properties.mjs
function BuildProperties(stack, context, schema2, value) {
  const required2 = IsRequired(schema2) ? schema2.required : [];
  const everyKey = exports_guard.Entries(schema2.properties).map(([key, schema3]) => {
    const notKey = exports_emit.Not(exports_emit.HasPropertyKey(value, exports_emit.Constant(key)));
    const isSchema = BuildSchemaPushStack(stack, context, schema3, exports_emit.Member(value, key));
    const addKey = context.AddKey(exports_emit.Constant(key));
    const guarded = context.UseUnevaluated() ? exports_emit.And(isSchema, addKey) : isSchema;
    const isProperty = required2.includes(key) ? guarded : exports_emit.Or(notKey, guarded);
    return IsExactOptional(required2, key) ? isProperty : exports_emit.Or(InexactOptionalBuild(value, key), isProperty);
  });
  return exports_emit.ReduceAnd(everyKey);
}
function CheckProperties(stack, context, schema2, value) {
  const required2 = IsRequired(schema2) ? schema2.required : [];
  const isProperties = exports_guard.Every(exports_guard.Entries(schema2.properties), 0, ([key, schema3]) => {
    const isProperty = !exports_guard.HasPropertyKey(value, key) || CheckSchemaPushStack(stack, context, schema3, value[key]) && context.AddKey(key);
    return IsExactOptional(required2, key) ? isProperty : InexactOptionalCheck(value, key) || isProperty;
  });
  return isProperties;
}
function ErrorProperties(stack, context, schemaPath, instancePath, schema2, value) {
  const required2 = IsRequired(schema2) ? schema2.required : [];
  const isProperties = exports_guard.EveryAll(exports_guard.Entries(schema2.properties), 0, ([key, schema3]) => {
    const nextSchemaPath = `${schemaPath}/properties/${key}`;
    const nextInstancePath = `${instancePath}/${key}`;
    const isProperty = () => !exports_guard.HasPropertyKey(value, key) || ErrorSchemaPushStack(stack, context, nextSchemaPath, nextInstancePath, schema3, value[key]) && context.AddKey(key);
    return IsExactOptional(required2, key) ? isProperty() : InexactOptionalCheck(value, key) || isProperty();
  });
  return isProperties;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/propertyNames.mjs
function BuildPropertyNames(stack, context, schema2, value) {
  const [key, _index] = [Unique(), Unique()];
  return exports_emit.Every(exports_emit.Keys(value), exports_emit.Constant(0), [key, _index], BuildSchema(stack, context, schema2.propertyNames, key));
}
function CheckPropertyNames(stack, context, schema2, value) {
  return exports_guard.Every(exports_guard.Keys(value), 0, (key, _index) => CheckSchema(stack, context, schema2.propertyNames, key));
}
function ErrorPropertyNames(stack, context, schemaPath, instancePath, schema2, value) {
  const propertyNames2 = [];
  const isPropertyNames = exports_guard.EveryAll(exports_guard.Keys(value), 0, (key, _index) => {
    const nextInstancePath = `${instancePath}/${key}`;
    const nextSchemaPath = `${schemaPath}/propertyNames`;
    const nextContext = new AccumulatedErrorContext;
    const isPropertyName = ErrorSchema(stack, nextContext, nextSchemaPath, nextInstancePath, schema2.propertyNames, key);
    if (!isPropertyName)
      propertyNames2.push(key);
    return isPropertyName;
  });
  return isPropertyNames || context.AddError({
    keyword: "propertyNames",
    schemaPath,
    instancePath,
    params: { propertyNames: propertyNames2 }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/recursiveRef.mjs
function BuildRecursiveRef(stack, context, schema2, value) {
  const target = stack.RecursiveRef(schema2) ?? false;
  return CreateFunction(stack, context, target, value);
}
function CheckRecursiveRef(stack, context, schema2, value) {
  const target = stack.RecursiveRef(schema2) ?? false;
  return IsSchema(target) && CheckSchema(stack, context, target, value);
}
function ErrorRecursiveRef(stack, context, _schemaPath, instancePath, schema2, value) {
  const target = stack.RecursiveRef(schema2) ?? false;
  return IsSchema(target) && ErrorSchema(stack, context, "#", instancePath, target, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/ref.mjs
function BuildRefStandard(stack, context, target, value) {
  const interior = exports_emit.ArrowFunction(["context", "value"], CreateFunction(stack, context, target, "value"));
  const exterior = exports_emit.ArrowFunction(["context", "value"], exports_emit.Statements([
    exports_emit.ConstDeclaration("nextContext", exports_emit.New("CheckContext", [])),
    exports_emit.ConstDeclaration("result", exports_emit.Call(interior, ["nextContext", "value"])),
    exports_emit.If("result", context.Merge("[nextContext]")),
    exports_emit.Return("result")
  ]));
  return exports_emit.Call(exterior, ["context", value]);
}
function BuildRefFast(stack, context, target, value) {
  return CreateFunction(stack, context, target, value);
}
function BuildRef(stack, context, schema2, value) {
  const target = stack.Ref(schema2) ?? false;
  return context.UseUnevaluated() ? BuildRefStandard(stack, context, target, value) : BuildRefFast(stack, context, target, value);
}
function CheckRef(stack, context, schema2, value) {
  const target = stack.Ref(schema2) ?? false;
  const nextContext = new CheckContext;
  const result = IsSchema(target) && CheckSchema(stack, nextContext, target, value);
  if (result)
    context.Merge([nextContext]);
  return result;
}
function ErrorRef(stack, context, _schemaPath, instancePath, schema2, value) {
  const target = stack.Ref(schema2) ?? false;
  const nextContext = new AccumulatedErrorContext;
  const result = IsSchema(target) && ErrorSchema(stack, nextContext, "#", instancePath, target, value);
  if (result)
    context.Merge([nextContext]);
  if (!result)
    nextContext.GetErrors().forEach((error) => context.AddError(error));
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/required.mjs
function BuildRequired(_stack, _context, schema2, value) {
  return exports_emit.ReduceAnd(schema2.required.map((key) => exports_emit.HasPropertyKey(value, exports_emit.Constant(key))));
}
function CheckRequired(_stack, _context, schema2, value) {
  return exports_guard.Every(schema2.required, 0, (key) => exports_guard.HasPropertyKey(value, key));
}
function ErrorRequired(_stack, context, schemaPath, instancePath, schema2, value) {
  const requiredProperties = [];
  const isRequired = exports_guard.EveryAll(schema2.required, 0, (key) => {
    const hasKey = exports_guard.HasPropertyKey(value, key);
    if (!hasKey)
      requiredProperties.push(key);
    return hasKey;
  });
  return isRequired || context.AddError({
    keyword: "required",
    schemaPath,
    instancePath,
    params: { requiredProperties }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/type.mjs
function BuildTypeName(_stack, _context, type2, value) {
  return exports_guard.IsEqual(type2, "object") ? exports_emit.IsObjectNotArray(value) : exports_guard.IsEqual(type2, "array") ? exports_emit.IsArray(value) : exports_guard.IsEqual(type2, "boolean") ? exports_emit.IsBoolean(value) : exports_guard.IsEqual(type2, "integer") ? exports_emit.IsInteger(value) : exports_guard.IsEqual(type2, "number") ? exports_emit.IsNumber(value) : exports_guard.IsEqual(type2, "null") ? exports_emit.IsNull(value) : exports_guard.IsEqual(type2, "string") ? exports_emit.IsString(value) : exports_guard.IsEqual(type2, "asyncIterator") ? exports_emit.IsAsyncIterator(value) : exports_guard.IsEqual(type2, "bigint") ? exports_emit.IsBigInt(value) : exports_guard.IsEqual(type2, "constructor") ? exports_emit.IsConstructor(value) : exports_guard.IsEqual(type2, "function") ? exports_emit.IsFunction(value) : exports_guard.IsEqual(type2, "iterator") ? exports_emit.IsIterator(value) : exports_guard.IsEqual(type2, "symbol") ? exports_emit.IsSymbol(value) : exports_guard.IsEqual(type2, "undefined") ? exports_emit.IsUndefined(value) : exports_guard.IsEqual(type2, "void") ? exports_emit.IsUndefined(value) : exports_emit.Constant(true);
}
function CheckTypeName(_stack, _context, type2, _schema, value) {
  return exports_guard.IsEqual(type2, "object") ? exports_guard.IsObjectNotArray(value) : exports_guard.IsEqual(type2, "array") ? exports_guard.IsArray(value) : exports_guard.IsEqual(type2, "boolean") ? exports_guard.IsBoolean(value) : exports_guard.IsEqual(type2, "integer") ? exports_guard.IsInteger(value) : exports_guard.IsEqual(type2, "number") ? exports_guard.IsNumber(value) : exports_guard.IsEqual(type2, "null") ? exports_guard.IsNull(value) : exports_guard.IsEqual(type2, "string") ? exports_guard.IsString(value) : exports_guard.IsEqual(type2, "asyncIterator") ? exports_guard.IsAsyncIterator(value) : exports_guard.IsEqual(type2, "bigint") ? exports_guard.IsBigInt(value) : exports_guard.IsEqual(type2, "constructor") ? exports_guard.IsConstructor(value) : exports_guard.IsEqual(type2, "function") ? exports_guard.IsFunction(value) : exports_guard.IsEqual(type2, "iterator") ? exports_guard.IsIterator(value) : exports_guard.IsEqual(type2, "symbol") ? exports_guard.IsSymbol(value) : exports_guard.IsEqual(type2, "undefined") ? exports_guard.IsUndefined(value) : exports_guard.IsEqual(type2, "void") ? exports_guard.IsUndefined(value) : true;
}
function BuildTypeNames(stack, context, typenames, value) {
  return exports_emit.ReduceOr(typenames.map((type2) => BuildTypeName(stack, context, type2, value)));
}
function CheckTypeNames(stack, context, types, schema2, value) {
  return types.some((type2) => CheckTypeName(stack, context, type2, schema2, value));
}
function BuildType(stack, context, schema2, value) {
  return exports_guard.IsArray(schema2.type) ? BuildTypeNames(stack, context, schema2.type, value) : BuildTypeName(stack, context, schema2.type, value);
}
function CheckType(stack, context, schema2, value) {
  return exports_guard.IsArray(schema2.type) ? CheckTypeNames(stack, context, schema2.type, schema2, value) : CheckTypeName(stack, context, schema2.type, schema2, value);
}
function ErrorType(stack, context, schemaPath, instancePath, schema2, value) {
  const isType = exports_guard.IsArray(schema2.type) ? CheckTypeNames(stack, context, schema2.type, schema2, value) : CheckTypeName(stack, context, schema2.type, schema2, value);
  return isType || context.AddError({
    keyword: "type",
    schemaPath,
    instancePath,
    params: { type: schema2.type }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/unevaluatedItems.mjs
function BuildUnevaluatedItems(stack, context, schema2, value) {
  const [index2, item] = [Unique(), Unique()];
  const indices = exports_emit.Call(exports_emit.Member("context", "GetIndices"), []);
  const hasIndex = exports_emit.Call(exports_emit.Member("indices", "has"), [index2]);
  const isSchema = BuildSchema(stack, context, schema2.unevaluatedItems, item);
  const addIndex = exports_emit.Call(exports_emit.Member("context", "AddIndex"), [index2]);
  const isEvery = exports_emit.Every(value, exports_emit.Constant(0), [item, index2], exports_emit.And(exports_emit.Or(hasIndex, isSchema), addIndex));
  return exports_emit.Call(exports_emit.ArrowFunction(["context"], exports_emit.Statements([
    exports_emit.ConstDeclaration("indices", indices),
    exports_emit.Return(isEvery)
  ])), ["context"]);
}
function CheckUnevaluatedItems(stack, context, schema2, value) {
  const indices = context.GetIndices();
  return exports_guard.Every(value, 0, (item, index2) => {
    return (indices.has(index2) || CheckSchema(stack, context, schema2.unevaluatedItems, item)) && context.AddIndex(index2);
  });
}
function ErrorUnevaluatedItems(stack, context, schemaPath, instancePath, schema2, value) {
  const indices = context.GetIndices();
  const unevaluatedItems2 = [];
  const isUnevaluatedItems = exports_guard.EveryAll(value, 0, (item, index2) => {
    const nextContext = new AccumulatedErrorContext;
    const isEvaluatedItem = (indices.has(index2) || ErrorSchema(stack, nextContext, schemaPath, instancePath, schema2.unevaluatedItems, item)) && context.AddIndex(index2);
    if (!isEvaluatedItem)
      unevaluatedItems2.push(index2);
    return isEvaluatedItem;
  });
  return isUnevaluatedItems || context.AddError({
    keyword: "unevaluatedItems",
    schemaPath,
    instancePath,
    params: { unevaluatedItems: unevaluatedItems2 }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/unevaluatedProperties.mjs
function BuildUnevaluatedProperties(stack, context, schema2, value) {
  const [key, prop] = [Unique(), Unique()];
  const keys = exports_emit.Call(exports_emit.Member("context", "GetKeys"), []);
  const hasKey = exports_emit.Call(exports_emit.Member("keys", "has"), [key]);
  const addKey = exports_emit.Call(exports_emit.Member("context", "AddKey"), [key]);
  const isSchema = BuildSchema(stack, context, schema2.unevaluatedProperties, prop);
  const isEvery = exports_emit.Every(exports_emit.Entries(value), exports_emit.Constant(0), [`[${key}, ${prop}]`, "_"], exports_emit.Or(hasKey, exports_emit.And(isSchema, addKey)));
  return exports_emit.Call(exports_emit.ArrowFunction(["context"], exports_emit.Statements([
    exports_emit.ConstDeclaration("keys", keys),
    exports_emit.Return(isEvery)
  ])), ["context"]);
}
function CheckUnevaluatedProperties(stack, context, schema2, value) {
  const keys = context.GetKeys();
  return exports_guard.Every(exports_guard.Entries(value), 0, ([key, prop]) => {
    return keys.has(key) || CheckSchema(stack, context, schema2.unevaluatedProperties, prop) && context.AddKey(key);
  });
}
function ErrorUnevaluatedProperties(stack, context, schemaPath, instancePath, schema2, value) {
  const keys = context.GetKeys();
  const unevaluatedProperties2 = [];
  const isUnevaluatedProperties = exports_guard.EveryAll(exports_guard.Entries(value), 0, ([key, prop]) => {
    const nextContext = new AccumulatedErrorContext;
    const isEvaluatedProperty = keys.has(key) || ErrorSchema(stack, nextContext, schemaPath, instancePath, schema2.unevaluatedProperties, prop) && context.AddKey(key);
    if (!isEvaluatedProperty)
      unevaluatedProperties2.push(key);
    return isEvaluatedProperty;
  });
  return isUnevaluatedProperties || context.AddError({
    keyword: "unevaluatedProperties",
    schemaPath,
    instancePath,
    params: { unevaluatedProperties: unevaluatedProperties2 }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/uniqueItems.mjs
function IsValid5(schema2) {
  return !exports_guard.IsEqual(schema2.uniqueItems, false);
}
function BuildUniqueItems(_stack, _context, schema2, value) {
  if (!IsValid5(schema2))
    return exports_emit.Constant(true);
  const set = exports_emit.Member(exports_emit.New("Set", [exports_emit.Call(exports_emit.Member(value, "map"), [exports_emit.Member("Hashing", "Hash")])]), "size");
  const isLength = exports_emit.Member(value, "length");
  return exports_emit.IsEqual(set, isLength);
}
function CheckUniqueItems(_stack, _context, schema2, value) {
  if (!IsValid5(schema2))
    return true;
  const set = new Set(value.map(exports_hash.Hash)).size;
  const isLength = value.length;
  return exports_guard.IsEqual(set, isLength);
}
function ErrorUniqueItems(_stack, context, schemaPath, instancePath, schema2, value) {
  if (!IsValid5(schema2))
    return true;
  const set = new Set;
  const duplicateItems = value.reduce((result, value2, index2) => {
    const hash = exports_hash.Hash(value2);
    if (set.has(hash))
      return [...result, index2];
    set.add(hash);
    return result;
  }, []);
  const isUniqueItems = exports_guard.IsEqual(duplicateItems.length, 0);
  return isUniqueItems || context.AddError({
    keyword: "uniqueItems",
    schemaPath,
    instancePath,
    params: { duplicateItems }
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/schema.mjs
function HasTypeName(schema2, typename) {
  return IsType(schema2) && (exports_guard.IsArray(schema2.type) && schema2.type.includes(typename) || exports_guard.IsEqual(schema2.type, typename));
}
function HasObjectType(schema2) {
  return HasTypeName(schema2, "object");
}
function HasObjectKeywords(schema2) {
  return IsSchemaObject(schema2) && (IsAdditionalProperties(schema2) || IsDependencies(schema2) || IsDependentRequired(schema2) || IsDependentSchemas(schema2) || IsProperties(schema2) || IsPatternProperties(schema2) || IsPropertyNames(schema2) || IsMinProperties(schema2) || IsMaxProperties(schema2) || IsRequired(schema2) || IsUnevaluatedProperties(schema2));
}
function HasArrayType(schema2) {
  return HasTypeName(schema2, "array");
}
function HasArrayKeywords(schema2) {
  return IsSchemaObject(schema2) && (IsAdditionalItems(schema2) || IsItems(schema2) || IsContains(schema2) || IsMaxContains(schema2) || IsMaxItems(schema2) || IsMinContains(schema2) || IsMinItems(schema2) || IsPrefixItems(schema2) || IsUnevaluatedItems(schema2) || IsUniqueItems(schema2));
}
function HasStringType(schema2) {
  return HasTypeName(schema2, "string");
}
function HasStringKeywords(schema2) {
  return IsSchemaObject(schema2) && (IsMinLength4(schema2) || IsMaxLength4(schema2) || IsFormat(schema2) || IsPattern(schema2));
}
function HasNumberType(schema2) {
  return HasTypeName(schema2, "number") || HasTypeName(schema2, "bigint");
}
function HasNumberKeywords(schema2) {
  return IsSchemaObject(schema2) && (IsMinimum(schema2) || IsMaximum(schema2) || IsExclusiveMaximum(schema2) || IsExclusiveMinimum(schema2) || IsMultipleOf2(schema2));
}
function BuildSchemaPushStack(stack, context, schema2, value) {
  return context.UseUnevaluated() ? exports_emit.And(exports_emit.And(context.Push(), BuildSchema(stack, context, schema2, value)), context.Pop()) : BuildSchema(stack, context, schema2, value);
}
function BuildSchema(stack, context, schema2, value) {
  stack.Push(schema2);
  const conditions = [];
  if (IsBooleanSchema(schema2))
    return BuildBooleanSchema(stack, context, schema2, value);
  if (IsType(schema2))
    conditions.push(BuildType(stack, context, schema2, value));
  if (HasObjectKeywords(schema2)) {
    const constraints = [];
    if (IsRequired(schema2))
      constraints.push(BuildRequired(stack, context, schema2, value));
    if (IsAdditionalProperties(schema2))
      constraints.push(BuildAdditionalProperties(stack, context, schema2, value));
    if (IsDependencies(schema2))
      constraints.push(BuildDependencies(stack, context, schema2, value));
    if (IsDependentRequired(schema2))
      constraints.push(BuildDependentRequired(stack, context, schema2, value));
    if (IsDependentSchemas(schema2))
      constraints.push(BuildDependentSchemas(stack, context, schema2, value));
    if (IsPatternProperties(schema2))
      constraints.push(BuildPatternProperties(stack, context, schema2, value));
    if (IsProperties(schema2))
      constraints.push(BuildProperties(stack, context, schema2, value));
    if (IsPropertyNames(schema2))
      constraints.push(BuildPropertyNames(stack, context, schema2, value));
    if (IsMinProperties(schema2))
      constraints.push(BuildMinProperties(stack, context, schema2, value));
    if (IsMaxProperties(schema2))
      constraints.push(BuildMaxProperties(stack, context, schema2, value));
    const reduced = exports_emit.ReduceAnd(constraints);
    const guarded = exports_emit.Or(exports_emit.Not(exports_emit.IsObjectNotArray(value)), reduced);
    conditions.push(HasObjectType(schema2) ? reduced : guarded);
  }
  if (HasArrayKeywords(schema2)) {
    const constraints = [];
    if (IsAdditionalItems(schema2))
      constraints.push(BuildAdditionalItems(stack, context, schema2, value));
    if (IsContains(schema2))
      constraints.push(BuildContains(stack, context, schema2, value));
    if (IsItems(schema2))
      constraints.push(BuildItems(stack, context, schema2, value));
    if (IsMaxContains(schema2))
      constraints.push(BuildMaxContains(stack, context, schema2, value));
    if (IsMaxItems(schema2))
      constraints.push(BuildMaxItems(stack, context, schema2, value));
    if (IsMinContains(schema2))
      constraints.push(BuildMinContains(stack, context, schema2, value));
    if (IsMinItems(schema2))
      constraints.push(BuildMinItems(stack, context, schema2, value));
    if (IsPrefixItems(schema2))
      constraints.push(BuildPrefixItems(stack, context, schema2, value));
    if (IsUniqueItems(schema2))
      constraints.push(BuildUniqueItems(stack, context, schema2, value));
    const reduced = exports_emit.ReduceAnd(constraints);
    const guarded = exports_emit.Or(exports_emit.Not(exports_emit.IsArray(value)), reduced);
    conditions.push(HasArrayType(schema2) ? reduced : guarded);
  }
  if (HasStringKeywords(schema2)) {
    const constraints = [];
    if (IsMaxLength4(schema2))
      constraints.push(BuildMaxLength(stack, context, schema2, value));
    if (IsMinLength4(schema2))
      constraints.push(BuildMinLength(stack, context, schema2, value));
    if (IsFormat(schema2))
      constraints.push(BuildFormat(stack, context, schema2, value));
    if (IsPattern(schema2))
      constraints.push(BuildPattern(stack, context, schema2, value));
    const reduced = exports_emit.ReduceAnd(constraints);
    const guarded = exports_emit.Or(exports_emit.Not(exports_emit.IsString(value)), reduced);
    conditions.push(HasStringType(schema2) ? reduced : guarded);
  }
  if (HasNumberKeywords(schema2)) {
    const constraints = [];
    if (IsExclusiveMaximum(schema2))
      constraints.push(BuildExclusiveMaximum(stack, context, schema2, value));
    if (IsExclusiveMinimum(schema2))
      constraints.push(BuildExclusiveMinimum(stack, context, schema2, value));
    if (IsMaximum(schema2))
      constraints.push(BuildMaximum(stack, context, schema2, value));
    if (IsMinimum(schema2))
      constraints.push(BuildMinimum(stack, context, schema2, value));
    if (IsMultipleOf2(schema2))
      constraints.push(BuildMultipleOf(stack, context, schema2, value));
    const reduced = exports_emit.ReduceAnd(constraints);
    const guarded = exports_emit.Or(exports_emit.Not(exports_emit.Or(exports_emit.IsNumber(value), exports_emit.IsBigInt(value))), reduced);
    conditions.push(HasNumberType(schema2) ? reduced : guarded);
  }
  if (IsRef(schema2))
    conditions.push(BuildRef(stack, context, schema2, value));
  if (IsRecursiveRef(schema2))
    conditions.push(BuildRecursiveRef(stack, context, schema2, value));
  if (IsDynamicRef(schema2))
    conditions.push(BuildDynamicRef(stack, context, schema2, value));
  if (IsGuard(schema2))
    conditions.push(BuildGuard(stack, context, schema2, value));
  if (IsConst(schema2))
    conditions.push(BuildConst(stack, context, schema2, value));
  if (IsEnum(schema2))
    conditions.push(BuildEnum(stack, context, schema2, value));
  if (IsIf(schema2))
    conditions.push(BuildIf(stack, context, schema2, value));
  if (IsNot(schema2))
    conditions.push(BuildNot(stack, context, schema2, value));
  if (IsAllOf(schema2))
    conditions.push(BuildAllOf(stack, context, schema2, value));
  if (IsAnyOf(schema2))
    conditions.push(BuildAnyOf(stack, context, schema2, value));
  if (IsOneOf(schema2))
    conditions.push(BuildOneOf(stack, context, schema2, value));
  if (IsUnevaluatedItems(schema2))
    conditions.push(exports_emit.Or(exports_emit.Not(exports_emit.IsArray(value)), BuildUnevaluatedItems(stack, context, schema2, value)));
  if (IsUnevaluatedProperties(schema2))
    conditions.push(exports_emit.Or(exports_emit.Not(exports_emit.IsObject(value)), BuildUnevaluatedProperties(stack, context, schema2, value)));
  if (IsRefine(schema2))
    conditions.push(BuildRefine(stack, context, schema2, value));
  const result = exports_emit.ReduceAnd(conditions);
  stack.Pop(schema2);
  return result;
}
function CheckSchemaPushStack(stack, context, schema2, value) {
  return context.Push() && CheckSchema(stack, context, schema2, value) && context.Pop();
}
function CheckSchema(stack, context, schema2, value) {
  stack.Push(schema2);
  const result = IsBooleanSchema(schema2) ? CheckBooleanSchema(stack, context, schema2, value) : (!IsType(schema2) || CheckType(stack, context, schema2, value)) && (!(exports_guard.IsObject(value) && !exports_guard.IsArray(value)) || (!IsRequired(schema2) || CheckRequired(stack, context, schema2, value)) && (!IsAdditionalProperties(schema2) || CheckAdditionalProperties(stack, context, schema2, value)) && (!IsDependencies(schema2) || CheckDependencies(stack, context, schema2, value)) && (!IsDependentRequired(schema2) || CheckDependentRequired(stack, context, schema2, value)) && (!IsDependentSchemas(schema2) || CheckDependentSchemas(stack, context, schema2, value)) && (!IsPatternProperties(schema2) || CheckPatternProperties(stack, context, schema2, value)) && (!IsProperties(schema2) || CheckProperties(stack, context, schema2, value)) && (!IsPropertyNames(schema2) || CheckPropertyNames(stack, context, schema2, value)) && (!IsMinProperties(schema2) || CheckMinProperties(stack, context, schema2, value)) && (!IsMaxProperties(schema2) || CheckMaxProperties(stack, context, schema2, value))) && (!exports_guard.IsArray(value) || (!IsAdditionalItems(schema2) || CheckAdditionalItems(stack, context, schema2, value)) && (!IsContains(schema2) || CheckContains(stack, context, schema2, value)) && (!IsItems(schema2) || CheckItems(stack, context, schema2, value)) && (!IsMaxContains(schema2) || CheckMaxContains(stack, context, schema2, value)) && (!IsMaxItems(schema2) || CheckMaxItems(stack, context, schema2, value)) && (!IsMinContains(schema2) || CheckMinContains(stack, context, schema2, value)) && (!IsMinItems(schema2) || CheckMinItems(stack, context, schema2, value)) && (!IsPrefixItems(schema2) || CheckPrefixItems(stack, context, schema2, value)) && (!IsUniqueItems(schema2) || CheckUniqueItems(stack, context, schema2, value))) && (!exports_guard.IsString(value) || (!IsMaxLength4(schema2) || CheckMaxLength(stack, context, schema2, value)) && (!IsMinLength4(schema2) || CheckMinLength(stack, context, schema2, value)) && (!IsFormat(schema2) || CheckFormat(stack, context, schema2, value)) && (!IsPattern(schema2) || CheckPattern(stack, context, schema2, value))) && (!(exports_guard.IsNumber(value) || exports_guard.IsBigInt(value)) || (!IsExclusiveMaximum(schema2) || CheckExclusiveMaximum(stack, context, schema2, value)) && (!IsExclusiveMinimum(schema2) || CheckExclusiveMinimum(stack, context, schema2, value)) && (!IsMaximum(schema2) || CheckMaximum(stack, context, schema2, value)) && (!IsMinimum(schema2) || CheckMinimum(stack, context, schema2, value)) && (!IsMultipleOf2(schema2) || CheckMultipleOf(stack, context, schema2, value))) && (!IsRef(schema2) || CheckRef(stack, context, schema2, value)) && (!IsRecursiveRef(schema2) || CheckRecursiveRef(stack, context, schema2, value)) && (!IsDynamicRef(schema2) || CheckDynamicRef(stack, context, schema2, value)) && (!IsGuard(schema2) || CheckGuard(stack, context, schema2, value)) && (!IsConst(schema2) || CheckConst(stack, context, schema2, value)) && (!IsEnum(schema2) || CheckEnum(stack, context, schema2, value)) && (!IsIf(schema2) || CheckIf(stack, context, schema2, value)) && (!IsNot(schema2) || CheckNot(stack, context, schema2, value)) && (!IsAllOf(schema2) || CheckAllOf(stack, context, schema2, value)) && (!IsAnyOf(schema2) || CheckAnyOf(stack, context, schema2, value)) && (!IsOneOf(schema2) || CheckOneOf(stack, context, schema2, value)) && (!IsUnevaluatedItems(schema2) || (!exports_guard.IsArray(value) || CheckUnevaluatedItems(stack, context, schema2, value))) && (!IsUnevaluatedProperties(schema2) || (!exports_guard.IsObject(value) || CheckUnevaluatedProperties(stack, context, schema2, value))) && (!IsRefine(schema2) || CheckRefine(stack, context, schema2, value));
  stack.Pop(schema2);
  return result;
}
function ErrorSchemaPushStack(stack, context, schemaPath, instancePath, schema2, value) {
  return context.Push() && ErrorSchema(stack, context, schemaPath, instancePath, schema2, value) && context.Pop();
}
function ErrorSchema(stack, context, schemaPath, instancePath, schema2, value) {
  stack.Push(schema2);
  const result = IsBooleanSchema(schema2) ? ErrorBooleanSchema(stack, context, schemaPath, instancePath, schema2, value) : !!(+(!IsType(schema2) || ErrorType(stack, context, schemaPath, instancePath, schema2, value)) & +(!(exports_guard.IsObject(value) && !exports_guard.IsArray(value)) || !!(+(!IsRequired(schema2) || ErrorRequired(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsAdditionalProperties(schema2) || ErrorAdditionalProperties(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsDependencies(schema2) || ErrorDependencies(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsDependentRequired(schema2) || ErrorDependentRequired(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsDependentSchemas(schema2) || ErrorDependentSchemas(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsPatternProperties(schema2) || ErrorPatternProperties(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsProperties(schema2) || ErrorProperties(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsPropertyNames(schema2) || ErrorPropertyNames(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMinProperties(schema2) || ErrorMinProperties(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMaxProperties(schema2) || ErrorMaxProperties(stack, context, schemaPath, instancePath, schema2, value)))) & +(!exports_guard.IsArray(value) || !!(+(!IsAdditionalItems(schema2) || ErrorAdditionalItems(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsContains(schema2) || ErrorContains(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsItems(schema2) || ErrorItems(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMaxContains(schema2) || ErrorMaxContains(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMaxItems(schema2) || ErrorMaxItems(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMinContains(schema2) || ErrorMinContains(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMinItems(schema2) || ErrorMinItems(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsPrefixItems(schema2) || ErrorPrefixItems(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsUniqueItems(schema2) || ErrorUniqueItems(stack, context, schemaPath, instancePath, schema2, value)))) & +(!exports_guard.IsString(value) || !!(+(!IsMaxLength4(schema2) || ErrorMaxLength(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMinLength4(schema2) || ErrorMinLength(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsFormat(schema2) || ErrorFormat(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsPattern(schema2) || ErrorPattern(stack, context, schemaPath, instancePath, schema2, value)))) & +(!(exports_guard.IsNumber(value) || exports_guard.IsBigInt(value)) || !!(+(!IsExclusiveMaximum(schema2) || ErrorExclusiveMaximum(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsExclusiveMinimum(schema2) || ErrorExclusiveMinimum(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMaximum(schema2) || ErrorMaximum(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMinimum(schema2) || ErrorMinimum(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsMultipleOf2(schema2) || ErrorMultipleOf(stack, context, schemaPath, instancePath, schema2, value)))) & +(!IsRef(schema2) || ErrorRef(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsRecursiveRef(schema2) || ErrorRecursiveRef(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsDynamicRef(schema2) || ErrorDynamicRef(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsGuard(schema2) || ErrorGuard(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsConst(schema2) || ErrorConst(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsEnum(schema2) || ErrorEnum(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsIf(schema2) || ErrorIf(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsNot(schema2) || ErrorNot(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsAllOf(schema2) || ErrorAllOf(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsAnyOf(schema2) || ErrorAnyOf(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsOneOf(schema2) || ErrorOneOf(stack, context, schemaPath, instancePath, schema2, value)) & +(!IsUnevaluatedItems(schema2) || (!exports_guard.IsArray(value) || ErrorUnevaluatedItems(stack, context, schemaPath, instancePath, schema2, value))) & +(!IsUnevaluatedProperties(schema2) || (!exports_guard.IsObject(value) || ErrorUnevaluatedProperties(stack, context, schemaPath, instancePath, schema2, value)))) && (!IsRefine(schema2) || ErrorRefine(stack, context, schemaPath, instancePath, schema2, value));
  stack.Pop(schema2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_functions.mjs
var functions = new Map;
function CreateCallExpression(context, _schema, hash, value) {
  return context.UseUnevaluated() ? exports_emit.Call(`check_${hash}`, ["context", value]) : exports_emit.Call(`check_${hash}`, [value]);
}
function CreateFunctionExpression(stack, context, schema2, hash) {
  const expression = BuildSchema(stack, context, schema2, "value");
  return context.UseUnevaluated() ? exports_emit.ConstDeclaration(`check_${hash}`, exports_emit.ArrowFunction(["context", "value"], expression)) : exports_emit.ConstDeclaration(`check_${hash}`, exports_emit.ArrowFunction(["value"], expression));
}
function ResetFunctions() {
  functions.clear();
}
function GetFunctions() {
  return [...functions.values()];
}
function CreateFunction(stack, context, schema2, value) {
  const hash = IsSchemaObject(schema2) ? exports_hash.Hash({ __baseURL: stack.BaseURL().href, ...schema2 }) : exports_hash.Hash(schema2);
  const call = CreateCallExpression(context, schema2, hash, value);
  if (functions.has(hash))
    return call;
  functions.set(hash, "");
  functions.set(hash, CreateFunctionExpression(stack, context, schema2, hash));
  return call;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/resolve/resolve.mjs
var exports_resolve = {};
__export(exports_resolve, {
  Ref: () => Ref,
  DynamicRef: () => DynamicRef
});
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/pointer/pointer.mjs
var exports_pointer = {};
__export(exports_pointer, {
  Set: () => Set4,
  Indices: () => Indices,
  Has: () => Has2,
  Get: () => Get3,
  Delete: () => Delete
});
function AssertNotRoot(indices) {
  if (indices.length === 0)
    throw Error("Cannot set root");
}
function AssertCanSet(value) {
  if (!exports_guard.IsObject(value))
    throw Error("Cannot set value");
}
function AssertIndex(index2) {
  if (exports_guard.IsUnsafePropertyKey(index2))
    throw Error("Pointer contains unsafe property key");
}
function AssertIndices(indices) {
  for (const index2 of indices)
    AssertIndex(index2);
}
function IsNumericIndex(index2) {
  return /^(0|[1-9]\d*)$/.test(index2);
}
function TakeIndexRight(indices) {
  return [
    indices.slice(0, indices.length - 1),
    indices.slice(indices.length - 1)[0]
  ];
}
function HasIndex(index2, value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, index2);
}
function GetIndex(index2, value) {
  return exports_guard.IsObject(value) && !exports_guard.IsUnsafePropertyKey(index2) ? value[index2] : undefined;
}
function GetIndices(indices, value) {
  return indices.reduce((value2, index2) => GetIndex(index2, value2), value);
}
function Indices(pointer) {
  if (exports_guard.IsEqual(pointer.length, 0))
    return [];
  const indices = pointer.split("/").map((index2) => index2.replace(/~1/g, "/").replace(/~0/g, "~"));
  return indices.length > 0 && indices[0] === "" ? indices.slice(1) : indices;
}
function Has2(value, pointer) {
  let current = value;
  return Indices(pointer).every((index2) => {
    if (!HasIndex(index2, current))
      return false;
    current = current[index2];
    return true;
  });
}
function Get3(value, pointer) {
  const indices = Indices(pointer);
  return GetIndices(indices, value);
}
function Set4(value, pointer, next) {
  const indices = Indices(pointer);
  AssertNotRoot(indices);
  AssertIndices(indices);
  const [head, index2] = TakeIndexRight(indices);
  const parent = GetIndices(head, value);
  AssertCanSet(parent);
  parent[index2] = next;
  return value;
}
function Delete(value, pointer) {
  const indices = Indices(pointer);
  AssertNotRoot(indices);
  AssertIndices(indices);
  const [head, index2] = TakeIndexRight(indices);
  const parent = GetIndices(head, value);
  AssertCanSet(parent);
  if (exports_guard.IsArray(parent) && IsNumericIndex(index2)) {
    parent.splice(+index2, 1);
  } else {
    delete parent[index2];
  }
  return value;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/resolve/ref.mjs
function MatchId(schema2, base, ref2) {
  if (schema2.$id === ref2.hash)
    return schema2;
  const absoluteId = new URL(schema2.$id, base.href);
  const absoluteRef = new URL(ref2.href, base.href);
  if (exports_guard.IsEqual(absoluteId.pathname, absoluteRef.pathname)) {
    return ref2.hash.startsWith("#") ? MatchHash(schema2, base, ref2) : schema2;
  }
  return;
}
function MatchAnchor(schema2, base, ref2) {
  const absoluteAnchor = new URL(`#${schema2.$anchor}`, base.href);
  const absoluteRef = new URL(ref2.href, base.href);
  return exports_guard.IsEqual(absoluteAnchor.href, absoluteRef.href) ? schema2 : undefined;
}
function MatchDynamicAnchor(schema2, base, ref2) {
  const absoluteAnchor = new URL(`#${schema2.$dynamicAnchor}`, base.href);
  const absoluteRef = new URL(ref2.href, base.href);
  return exports_guard.IsEqual(absoluteAnchor.href, absoluteRef.href) ? schema2 : undefined;
}
function MatchHash(schema2, _base, ref2) {
  if (ref2.href.endsWith("#"))
    return schema2;
  if (!ref2.hash.startsWith("#"))
    return;
  const fragment = decodeURIComponent(ref2.hash.slice(1));
  if (!fragment.startsWith("/"))
    return;
  return exports_pointer.Get(schema2, fragment);
}
function Match2(schema2, base, ref2) {
  if (IsId(schema2)) {
    const result = MatchId(schema2, base, ref2);
    if (!exports_guard.IsUndefined(result))
      return result;
  }
  if (IsAnchor(schema2)) {
    const result = MatchAnchor(schema2, base, ref2);
    if (!exports_guard.IsUndefined(result))
      return result;
  }
  if (IsDynamicAnchor(schema2)) {
    const result = MatchDynamicAnchor(schema2, base, ref2);
    if (!exports_guard.IsUndefined(result))
      return result;
  }
  return MatchHash(schema2, base, ref2);
}
function FromArray2(schema2, base, ref2) {
  return schema2.reduce((result, item) => {
    const match = FromValue2(item, base, ref2);
    return !exports_guard.IsUndefined(match) ? match : result;
  }, undefined);
}
function FromObject2(schema2, base, ref2) {
  return exports_guard.Keys(schema2).reduce((result, key) => {
    const match = FromValue2(schema2[key], base, ref2);
    return !exports_guard.IsUndefined(match) ? match : result;
  }, undefined);
}
function FromValue2(schema2, base, ref2) {
  const nextBase = IsSchemaObject(schema2) && IsId(schema2) ? new URL(schema2.$id, base.href) : base;
  if (IsSchemaObject(schema2)) {
    const result = Match2(schema2, nextBase, ref2);
    if (!exports_guard.IsUndefined(result))
      return result;
  }
  if (exports_guard.IsArray(schema2))
    return FromArray2(schema2, nextBase, ref2);
  if (exports_guard.IsObject(schema2))
    return FromObject2(schema2, nextBase, ref2);
  return;
}
function Ref(schema2, ref2) {
  const defaultBase = new URL("http://unknown/");
  const initialBase = IsId(schema2) ? new URL(schema2.$id, defaultBase.href) : defaultBase;
  const initialRef = new URL(ref2, initialBase.href);
  return FromValue2(schema2, initialBase, initialRef);
}
function DynamicRef(root, base, dynamicRef2, dynamicAnchors) {
  const fragmentTarget = dynamicRef2.$dynamicRef.startsWith("#") ? Ref(base, dynamicRef2.$dynamicRef) : Ref(root, dynamicRef2.$dynamicRef);
  if (exports_guard.IsUndefined(fragmentTarget))
    return;
  if (!IsSchemaObject(fragmentTarget) || !IsDynamicAnchor(fragmentTarget))
    return fragmentTarget;
  const fragment = new URL(dynamicRef2.$dynamicRef, "http://unknown/").hash;
  if (fragment.startsWith("#/"))
    return fragmentTarget;
  const anchorTarget = dynamicAnchors.find((anchor2) => anchor2.$dynamicAnchor === fragmentTarget.$dynamicAnchor);
  return anchorTarget ?? fragmentTarget;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/engine/_stack.mjs
var __classPrivateFieldGet = function(receiver, state2, kind, f) {
  if (kind === "a" && !f)
    throw new TypeError("Private accessor was defined without a getter");
  if (typeof state2 === "function" ? receiver !== state2 || !f : !state2.has(receiver))
    throw new TypeError("Cannot read private member from an object whose class did not declare it");
  return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state2.get(receiver);
};
var _Stack_instances;
var _Stack_PushResourceAnchors;
var _Stack_PopResourceAnchors;
var _Stack_FromContext;
var _Stack_FromRef;

class Stack {
  constructor(context, schema2) {
    _Stack_instances.add(this);
    this.context = context;
    this.schema = schema2;
    this.ids = [];
    this.anchors = [];
    this.recursiveAnchors = [];
    this.dynamicAnchors = [];
  }
  BaseURL() {
    return this.ids.reduce((result, schema2) => new URL(schema2.$id, result), new URL("http://unknown"));
  }
  Base() {
    return this.ids[this.ids.length - 1] ?? this.schema;
  }
  Push(schema2) {
    if (!IsSchemaObject(schema2))
      return;
    if (IsId(schema2)) {
      this.ids.push(schema2);
      __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_PushResourceAnchors).call(this, schema2);
    }
    if (IsAnchor(schema2))
      this.anchors.push(schema2);
    if (IsRecursiveAnchorTrue(schema2))
      this.recursiveAnchors.push(schema2);
    if (IsDynamicAnchor(schema2))
      this.dynamicAnchors.push(schema2);
  }
  Pop(schema2) {
    if (!IsSchemaObject(schema2))
      return;
    if (IsId(schema2)) {
      this.ids.pop();
      __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_PopResourceAnchors).call(this, schema2);
    }
    if (IsAnchor(schema2))
      this.anchors.pop();
    if (IsRecursiveAnchorTrue(schema2))
      this.recursiveAnchors.pop();
    if (IsDynamicAnchor(schema2))
      this.dynamicAnchors.pop();
  }
  Ref(ref3) {
    return __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_FromContext).call(this, ref3) ?? __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_FromRef).call(this, ref3);
  }
  RecursiveRef(recursiveRef2) {
    return IsRecursiveAnchorTrue(this.Base()) ? exports_resolve.Ref(this.recursiveAnchors[0], recursiveRef2.$recursiveRef) : exports_resolve.Ref(this.Base(), recursiveRef2.$recursiveRef);
  }
  DynamicRef(dynamicRef2) {
    const root = this.schema;
    return exports_resolve.DynamicRef(root, this.Base(), dynamicRef2, this.dynamicAnchors);
  }
}
_Stack_instances = new WeakSet, _Stack_PushResourceAnchors = function _Stack_PushResourceAnchors2(schema2, isRoot = true) {
  if (!IsSchemaObject(schema2))
    return;
  const current = schema2;
  if (!isRoot && IsId(current))
    return;
  if (!isRoot && IsDynamicAnchor(current))
    this.dynamicAnchors.push(current);
  for (const key of exports_guard.Keys(current))
    __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_PushResourceAnchors2).call(this, current[key], false);
}, _Stack_PopResourceAnchors = function _Stack_PopResourceAnchors2(schema2, isRoot = true) {
  if (!IsSchemaObject(schema2))
    return;
  const current = schema2;
  if (!isRoot && IsId(current))
    return;
  if (!isRoot && IsDynamicAnchor(current))
    this.dynamicAnchors.pop();
  for (const key of exports_guard.Keys(current))
    __classPrivateFieldGet(this, _Stack_instances, "m", _Stack_PopResourceAnchors2).call(this, current[key], false);
}, _Stack_FromContext = function _Stack_FromContext2(ref3) {
  return exports_guard.HasPropertyKey(this.context, ref3.$ref) ? this.context[ref3.$ref] : undefined;
}, _Stack_FromRef = function _Stack_FromRef2(ref3) {
  const root = this.schema;
  return !ref3.$ref.startsWith("#") ? exports_resolve.Ref(root, ref3.$ref) : exports_resolve.Ref(this.Base(), ref3.$ref);
};
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/environment/environment.mjs
var exports_environment = {};
__export(exports_environment, {
  Evaluate: () => Evaluate,
  CanEvaluate: () => CanEvaluate
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/environment/evaluate.mjs
var supported = undefined;
function TryEvaluate() {
  try {
    Evaluate("null")();
    return true;
  } catch {
    return false;
  }
}
function CanEvaluate() {
  if (exports_guard.IsUndefined(supported))
    supported = TryEvaluate();
  return supported && exports_settings.Get().useAcceleration;
}
function Evaluate(...args) {
  return new globalThis.Function(...args);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/build.mjs
function CreateCode(build) {
  const functions2 = build.Functions().join(`;
`);
  const statements = build.UseUnevaluated() ? ["const context = new CheckContext({}, {})", `return ${build.Entry()}`] : [`return ${build.Entry()}`];
  return `${functions2}; return (value) => { ${statements.join("; ")} }`;
}
function CreateEvaluatedCheck(build, code) {
  const factory = exports_environment.Evaluate("CheckContext", "Guard", "Format", "Hashing", build.External().identifier, code);
  return factory(CheckContext, exports_guard, exports_format, exports_hash, build.External().variables);
}
function CreateDynamicCheck(build) {
  const stack = new Stack(build.Context(), build.Schema());
  const context = new CheckContext;
  return (value) => CheckSchema(stack, context, build.Schema(), value);
}
function CreateCheck(build, code) {
  return exports_environment.CanEvaluate() ? CreateEvaluatedCheck(build, code) : CreateDynamicCheck(build);
}

class EvaluateResult {
  constructor(isAccelerated, code, check) {
    this.isAccelerated = isAccelerated;
    this.code = code;
    this.check = check;
  }
  IsAccelerated() {
    return this.isAccelerated;
  }
  Code() {
    return this.code;
  }
  Check(value) {
    return this.check(value);
  }
}

class BuildResult {
  constructor(context, schema3, external, functions2, entry, useUnevaluated) {
    this.context = context;
    this.schema = schema3;
    this.external = external;
    this.functions = functions2;
    this.entry = entry;
    this.useUnevaluated = useUnevaluated;
  }
  Context() {
    return this.context;
  }
  Schema() {
    return this.schema;
  }
  UseUnevaluated() {
    return this.useUnevaluated;
  }
  External() {
    return this.external;
  }
  Functions() {
    return this.functions;
  }
  Entry() {
    return this.entry;
  }
  Evaluate() {
    const code = CreateCode(this);
    const check = CreateCheck(this, code);
    return new EvaluateResult(exports_environment.CanEvaluate(), code, check);
  }
}
function Build(...args) {
  const [context, schema3] = exports_arguments.Match(args, {
    2: (context2, schema4) => [context2, schema4],
    1: (schema4) => [{}, schema4]
  });
  ResetExternal();
  ResetFunctions();
  const stack = new Stack(context, schema3);
  const build = new BuildContext(HasUnevaluated(context, schema3));
  const call = CreateFunction(stack, build, schema3, "value");
  const functions2 = GetFunctions();
  const externals = GetExternal();
  return new BuildResult(context, schema3, externals, functions2, call, build.UseUnevaluated());
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/locale/en_US.mjs
function en_US(error) {
  switch (error.keyword) {
    case "additionalProperties":
      return "must not have additional properties";
    case "anyOf":
      return "must match a schema in anyOf";
    case "boolean":
      return "schema is false";
    case "const":
      return "must be equal to constant";
    case "contains":
      return "must contain at least 1 valid item";
    case "dependencies":
      return `must have properties ${error.params.dependencies.join(", ")} when property ${error.params.property} is present`;
    case "dependentRequired":
      return `must have properties ${error.params.dependencies.join(", ")} when property ${error.params.property} is present`;
    case "enum":
      return "must be equal to one of the allowed values";
    case "exclusiveMaximum":
      return `must be ${error.params.comparison} ${error.params.limit}`;
    case "exclusiveMinimum":
      return `must be ${error.params.comparison} ${error.params.limit}`;
    case "format":
      return `must match format "${error.params.format}"`;
    case "if":
      return `must match "${error.params.failingKeyword}" schema`;
    case "maxItems":
      return `must not have more than ${error.params.limit} items`;
    case "maxLength":
      return `must not have more than ${error.params.limit} characters`;
    case "maxProperties":
      return `must not have more than ${error.params.limit} properties`;
    case "maximum":
      return `must be ${error.params.comparison} ${error.params.limit}`;
    case "minItems":
      return `must not have fewer than ${error.params.limit} items`;
    case "minLength":
      return `must not have fewer than ${error.params.limit} characters`;
    case "minProperties":
      return `must not have fewer than ${error.params.limit} properties`;
    case "minimum":
      return `must be ${error.params.comparison} ${error.params.limit}`;
    case "multipleOf":
      return `must be multiple of ${error.params.multipleOf}`;
    case "not":
      return "must not be valid";
    case "oneOf":
      return "must match exactly one schema in oneOf";
    case "pattern":
      return `must match pattern "${error.params.pattern}"`;
    case "propertyNames":
      return `property names ${error.params.propertyNames.join(", ")} are invalid`;
    case "required":
      return `must have required properties ${error.params.requiredProperties.join(", ")}`;
    case "type":
      return typeof error.params.type === "string" ? `must be ${error.params.type}` : `must be either ${error.params.type.join(" or ")}`;
    case "unevaluatedItems":
      return "must not have unevaluated items";
    case "unevaluatedProperties":
      return "must not have unevaluated properties";
    case "uniqueItems":
      return `must not have duplicate items`;
    case "~guard":
      return `must match check function`;
    case "~refine":
      return error.params.message;
    default:
      return "an unknown validation error occurred";
  }
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/locale/_config.mjs
var locale = en_US;
function Get4() {
  return locale;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/errors.mjs
function Errors(...args) {
  const [context, schema3, value] = exports_arguments.Match(args, {
    3: (context2, schema4, value2) => [context2, schema4, value2],
    2: (schema4, value2) => [{}, schema4, value2]
  });
  const settings2 = exports_settings.Get();
  const locale2 = Get4();
  const errors = [];
  const stack = new Stack(context, schema3);
  const errorContext = new ErrorContext((error) => {
    if (exports_guard.IsGreaterEqualThan(errors.length, settings2.maxErrors))
      return;
    return errors.push({ ...error, message: locale2(error) });
  });
  const result = ErrorSchema(stack, errorContext, "#", "", schema3, value);
  return [result, errors];
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/schema/check.mjs
function Check(...args) {
  const [context, schema3, value] = exports_arguments.Match(args, {
    3: (context2, schema4, value2) => [context2, schema4, value2],
    2: (schema4, value2) => [{}, schema4, value2]
  });
  const stack = new Stack(context, schema3);
  const checkContext = new CheckContext;
  return CheckSchema(stack, checkContext, schema3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/compile/code.mjs
function TsIgnore() {
  return `// @ts-ignore`;
}
function Separator() {
  return ``;
}
function ImportSection(build2) {
  const context = build2.UseUnevaluated() ? [`import { CheckContext } from "typebox/schema"`] : [];
  const hashing = `import { Hashing } from "typebox/system"`;
  const format4 = `import { Format } from "typebox/format"`;
  const guard = `import { Guard } from "typebox/guard"`;
  return [...context, hashing, format4, guard];
}
function ExternalSection(build2) {
  const { identifier } = build2.External();
  return [
    Separator(),
    TsIgnore(),
    `let ${identifier} = []`,
    Separator(),
    TsIgnore(),
    `export function SetExternal(external) { ${identifier} = external.variables }`
  ];
}
function FunctionSection(build2) {
  return build2.Functions().map((func) => [Separator(), TsIgnore(), `${func};`].join(`
`));
}
function ExportSection(build2) {
  const body = build2.UseUnevaluated() ? `const context = new CheckContext({}, {}); return ${build2.Entry()}` : `return ${build2.Entry()}`;
  return [
    Separator(),
    TsIgnore(),
    `export function Check(value) { ${body} }`
  ];
}
function Code(...args) {
  const [context, type3] = exports_arguments.Match(args, {
    2: (context2, type4) => [context2, type4],
    1: (type4) => [{}, type4]
  });
  const build2 = Build(context, type3);
  const code = [...ImportSection(build2), ...ExternalSection(build2), ...FunctionSection(build2), ...ExportSection(build2)].join(`
`);
  return { External: build2.External(), Code: code };
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/memory.mjs
var exports_memory = {};
__export(exports_memory, {
  Update: () => Update,
  Metrics: () => Metrics,
  Discard: () => Discard,
  Create: () => Create,
  Clone: () => Clone,
  Assign: () => Assign
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/metrics.mjs
var Metrics = {
  assign: 0,
  create: 0,
  clone: 0,
  discard: 0,
  update: 0
};

// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/assign.mjs
function Assign(left, right) {
  Metrics.assign += 1;
  return { ...left, ...right };
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/clone.mjs
function IsGuard2(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~guard");
}
function FromGuard(value) {
  return value;
}
function FromArray3(value) {
  return value.map((value2) => FromValue3(value2));
}
function FromObject3(value) {
  const result = {};
  const descriptors = Object.getOwnPropertyDescriptors(value);
  for (const key of Object.keys(descriptors)) {
    const descriptor = descriptors[key];
    if (exports_guard.HasPropertyKey(descriptor, "value")) {
      Object.defineProperty(result, key, { ...descriptor, value: FromValue3(descriptor.value) });
    }
  }
  return result;
}
function FromRegExp2(value) {
  return new RegExp(value.source, value.flags);
}
function FromUnknown(value) {
  return value;
}
function FromValue3(value) {
  return value instanceof RegExp ? FromRegExp2(value) : IsGuard2(value) ? FromGuard(value) : exports_guard.IsArray(value) ? FromArray3(value) : exports_guard.IsObject(value) ? FromObject3(value) : FromUnknown(value);
}
function Clone(value) {
  Metrics.clone += 1;
  return FromValue3(value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/create.mjs
function MergeHidden(left, right) {
  for (const key of Object.keys(right)) {
    Object.defineProperty(left, key, {
      configurable: true,
      writable: true,
      enumerable: false,
      value: right[key]
    });
  }
  return left;
}
function Merge(left, right) {
  return { ...left, ...right };
}
function Create(hidden, enumerable, options = {}) {
  Metrics.create += 1;
  const settings2 = exports_settings.Get();
  const withOptions = Merge(enumerable, options);
  const withHidden = settings2.enumerableKind ? Merge(withOptions, hidden) : MergeHidden(withOptions, hidden);
  return settings2.immutableTypes ? Object.freeze(withHidden) : withHidden;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/discard.mjs
function Discard(value, propertyKeys) {
  Metrics.discard += 1;
  const result = {};
  const descriptors = Object.getOwnPropertyDescriptors(Clone(value));
  const keysToDiscard = new Set(propertyKeys);
  for (const key of Object.keys(descriptors)) {
    if (keysToDiscard.has(key))
      continue;
    Object.defineProperty(result, key, descriptors[key]);
  }
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/system/memory/update.mjs
function Update(current, hidden, enumerable) {
  Metrics.update += 1;
  const settings2 = exports_settings.Get();
  const result = Clone(current);
  for (const key of Object.keys(hidden)) {
    Object.defineProperty(result, key, {
      configurable: true,
      writable: true,
      enumerable: settings2.enumerableKind,
      value: hidden[key]
    });
  }
  for (const key of Object.keys(enumerable)) {
    Object.defineProperty(result, key, {
      configurable: true,
      enumerable: true,
      writable: true,
      value: enumerable[key]
    });
  }
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/schema.mjs
function IsKind(value, kind) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.IsEqual(value["~kind"], kind);
}
function IsSchema2(value) {
  return exports_guard.IsObject(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/_optional.mjs
function IsOptionalAddAction(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "type") && exports_guard.IsEqual(value["~kind"], "OptionalAddAction") && IsSchema2(value.type);
}
function IsOptionalRemoveAction(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "type") && exports_guard.IsEqual(value["~kind"], "OptionalRemoveAction") && IsSchema2(value.type);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/_readonly.mjs
function IsReadonlyAddAction(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "type") && exports_guard.IsEqual(value["~kind"], "ReadonlyAddAction") && IsSchema2(value.type);
}
function IsReadonlyRemoveAction(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "type") && exports_guard.IsEqual(value["~kind"], "ReadonlyRemoveAction") && IsSchema2(value.type);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/deferred.mjs
function Deferred(action, parameters, options) {
  return exports_memory.Create({ "~kind": "Deferred" }, { action, parameters, options }, {});
}
function IsDeferred(value) {
  return IsKind(value, "Deferred");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/promise.mjs
function _Promise_(item, options) {
  return exports_memory.Create({ ["~kind"]: "Promise" }, { type: "promise", item }, options);
}
function IsPromise(value) {
  return IsKind(value, "Promise");
}
function PromiseOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "item"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/_immutable.mjs
function ImmutableAdd(type3) {
  return exports_memory.Update(type3, { "~immutable": true }, {});
}
function Immutable(type3) {
  return ImmutableAdd(type3);
}
function IsImmutable(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "~immutable");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/_optional.mjs
function OptionalRemove(type3) {
  const result = exports_memory.Discard(type3, ["~optional"]);
  return result;
}
function OptionalAdd(type3) {
  return exports_memory.Update(type3, { "~optional": true }, {});
}
function Optional(type3) {
  return OptionalAdd(type3);
}
function IsOptional(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "~optional");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/_readonly.mjs
function ReadonlyRemove(type3) {
  return exports_memory.Discard(type3, ["~readonly"]);
}
function ReadonlyAdd(type3) {
  return exports_memory.Update(type3, { "~readonly": true }, {});
}
function Readonly(type3) {
  return ReadonlyAdd(type3);
}
function IsReadonly(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "~readonly");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/base.mjs
function BaseProperty(value) {
  return {
    enumerable: exports_settings.Get().enumerableKind,
    writable: false,
    configurable: false,
    value
  };
}

class Base {
  constructor() {
    globalThis.Object.defineProperty(this, "~kind", BaseProperty("Base"));
    globalThis.Object.defineProperty(this, "~guard", BaseProperty({
      check: (value) => this.Check(value),
      errors: (value) => this.Errors(value)
    }));
  }
  Check(_value) {
    return true;
  }
  Errors(_value) {
    return [];
  }
  Convert(value) {
    return value;
  }
  Clean(value) {
    return value;
  }
  Default(value) {
    return value;
  }
  Create() {
    throw new Error("Create not implemented");
  }
  Clone() {
    throw Error("Clone not implemented");
  }
}
function IsBase(value) {
  return IsKind(value, "Base");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/array.mjs
function _Array_(items3, options) {
  return exports_memory.Create({ "~kind": "Array" }, { type: "array", items: items3 }, options);
}
function IsArray3(value) {
  return IsKind(value, "Array");
}
function ArrayOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "items"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/async_iterator.mjs
function AsyncIterator(iteratorItems, options) {
  return exports_memory.Create({ "~kind": "AsyncIterator" }, { type: "asyncIterator", iteratorItems }, options);
}
function IsAsyncIterator3(value) {
  return IsKind(value, "AsyncIterator");
}
function AsyncIteratorOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "iteratorItems"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/constructor.mjs
function Constructor(parameters, instanceType, options = {}) {
  return exports_memory.Create({ "~kind": "Constructor" }, { type: "constructor", parameters, instanceType }, options);
}
function IsConstructor3(value) {
  return IsKind(value, "Constructor");
}
function ConstructorOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "parameters", "instanceType"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/function.mjs
function _Function_(parameters, returnType, options = {}) {
  return exports_memory.Create({ ["~kind"]: "Function" }, { type: "function", parameters, returnType }, options);
}
function IsFunction3(value) {
  return IsKind(value, "Function");
}
function FunctionOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "parameters", "returnType"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/ref.mjs
function Ref2(ref4, options) {
  return exports_memory.Create({ ["~kind"]: "Ref" }, { $ref: ref4 }, options);
}
function IsRef2(value) {
  return IsKind(value, "Ref");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/generic.mjs
function Generic(parameters, expression) {
  return exports_memory.Create({ "~kind": "Generic" }, { type: "generic", parameters, expression });
}
function IsGeneric(value) {
  return IsKind(value, "Generic");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/any.mjs
function Any(options) {
  return exports_memory.Create({ ["~kind"]: "Any" }, {}, options);
}
function IsAny(value) {
  return IsKind(value, "Any");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/never.mjs
var NeverPattern = "(?!)";
function Never(options) {
  return exports_memory.Create({ "~kind": "Never" }, { not: {} }, options);
}
function IsNever(value) {
  return IsKind(value, "Never");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/properties.mjs
function RequiredArray(properties3) {
  return exports_guard.Keys(properties3).filter((key) => !IsOptional(properties3[key]));
}
function PropertyKeys(properties3) {
  return exports_guard.Keys(properties3);
}
function PropertyValues(properties3) {
  return exports_guard.Values(properties3);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/object.mjs
function _Object_(properties3, options = {}) {
  const requiredKeys = RequiredArray(properties3);
  const required3 = requiredKeys.length > 0 ? { required: requiredKeys } : {};
  return exports_memory.Create({ "~kind": "Object" }, { type: "object", ...required3, properties: properties3 }, options);
}
function IsObject3(value) {
  return IsKind(value, "Object");
}
function ObjectOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "properties", "required"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/union.mjs
function Union(anyOf3, options = {}) {
  return exports_memory.Create({ "~kind": "Union" }, { anyOf: anyOf3 }, options);
}
function IsUnion(value) {
  return IsKind(value, "Union");
}
function UnionOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "anyOf"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/unknown.mjs
function Unknown(options) {
  return exports_memory.Create({ ["~kind"]: "Unknown" }, {}, options);
}
function IsUnknown(value) {
  return IsKind(value, "Unknown");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/cyclic.mjs
function Cyclic($defs, $ref, options) {
  const defs2 = exports_guard.Keys($defs).reduce((result, key) => {
    return { ...result, [key]: exports_memory.Update($defs[key], {}, { $id: key }) };
  }, {});
  return exports_memory.Create({ ["~kind"]: "Cyclic" }, { $defs: defs2, $ref }, options);
}
function IsCyclic(value) {
  return IsKind(value, "Cyclic");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/unsafe.mjs
function IsUnsafe(value) {
  return exports_guard.IsObjectNotArray(value) && exports_guard.HasPropertyKey(value, "~unsafe") && exports_guard.IsNull(value["~unsafe"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/infer.mjs
function IsInfer(value) {
  return IsKind(value, "Infer");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/enum.mjs
function IsEnum2(value) {
  return IsKind(value, "Enum");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/intersect.mjs
function Intersect(types2, options = {}) {
  return exports_memory.Create({ "~kind": "Intersect" }, { allOf: types2 }, options);
}
function IsIntersect(value) {
  return IsKind(value, "Intersect");
}
function IntersectOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "allOf"]);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/_codec.mjs
function IsCodec(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "~codec") && exports_guard.IsObject(value["~codec"]) && exports_guard.HasPropertyKey(value["~codec"], "encode") && exports_guard.HasPropertyKey(value["~codec"], "decode");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/bigint.mjs
var BigIntPattern = "-?(?:0|[1-9][0-9]*)n";
function BigInt2(options) {
  return exports_memory.Create({ "~kind": "BigInt" }, { type: "bigint" }, options);
}
function IsBigInt3(value) {
  return IsKind(value, "BigInt");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/boolean.mjs
function IsBoolean4(value) {
  return IsKind(value, "Boolean");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/integer.mjs
var IntegerPattern = "-?(?:0|[1-9][0-9]*)";
function Integer(options) {
  return exports_memory.Create({ "~kind": "Integer" }, { type: "integer" }, options);
}
function IsInteger3(value) {
  return IsKind(value, "Integer");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/iterator.mjs
function Iterator(iteratorItems, options) {
  return exports_memory.Create({ "~kind": "Iterator" }, { type: "iterator", iteratorItems }, options);
}
function IsIterator3(value) {
  return IsKind(value, "Iterator");
}
function IteratorOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "iteratorItems"]);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/literal.mjs
class InvalidLiteralValue extends Error {
  constructor(value) {
    super(`Invalid Literal value`);
    Object.defineProperty(this, "cause", {
      value: { value },
      writable: false,
      configurable: false,
      enumerable: false
    });
  }
}
function LiteralTypeName(value) {
  return exports_guard.IsBigInt(value) ? "bigint" : exports_guard.IsBoolean(value) ? "boolean" : exports_guard.IsNumber(value) ? "number" : exports_guard.IsString(value) ? "string" : (() => {
    throw new InvalidLiteralValue(value);
  })();
}
function Literal(value, options) {
  return exports_memory.Create({ "~kind": "Literal" }, { type: LiteralTypeName(value), const: value }, options);
}
function IsLiteralValue(value) {
  return exports_guard.IsBigInt(value) || exports_guard.IsBoolean(value) || exports_guard.IsNumber(value) || exports_guard.IsString(value);
}
function IsLiteralBigInt(value) {
  return IsLiteral(value) && exports_guard.IsBigInt(value.const);
}
function IsLiteralBoolean(value) {
  return IsLiteral(value) && exports_guard.IsBoolean(value.const);
}
function IsLiteralNumber(value) {
  return IsLiteral(value) && exports_guard.IsNumber(value.const);
}
function IsLiteralString(value) {
  return IsLiteral(value) && exports_guard.IsString(value.const);
}
function IsLiteral(value) {
  return IsKind(value, "Literal");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/null.mjs
function Null(options) {
  return exports_memory.Create({ "~kind": "Null" }, { type: "null" }, options);
}
function IsNull3(value) {
  return IsKind(value, "Null");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/number.mjs
var NumberPattern = "-?(?:0|[1-9][0-9]*)(?:.[0-9]+)?";
function Number2(options) {
  return exports_memory.Create({ "~kind": "Number" }, { type: "number" }, options);
}
function IsNumber4(value) {
  return IsKind(value, "Number");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/symbol.mjs
function Symbol2(options) {
  return exports_memory.Create({ "~kind": "Symbol" }, { type: "symbol" }, options);
}
function IsSymbol3(value) {
  return IsKind(value, "Symbol");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/string.mjs
var StringPattern = ".*";
function String2(options) {
  return exports_memory.Create({ "~kind": "String" }, { type: "string" }, options);
}
function IsString4(value) {
  return IsKind(value, "String");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/patterns/pattern.mjs
function ParsePatternIntoTypes(pattern3) {
  const parsed = Pattern(pattern3);
  const result = exports_guard.IsEqual(parsed.length, 2) ? parsed[0] : [];
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/template_literal/is_finite.mjs
function FromLiteral(_value) {
  return true;
}
function FromTypesReduce(types2) {
  return exports_guard.TakeLeft(types2, (left, right) => FromType(left) ? FromTypesReduce(right) : false, () => true);
}
function FromTypes(types2) {
  const result = exports_guard.IsEqual(types2.length, 0) ? false : FromTypesReduce(types2);
  return result;
}
function FromType(type3) {
  return IsUnion(type3) ? FromTypes(type3.anyOf) : IsLiteral(type3) ? FromLiteral(type3.const) : false;
}
function IsTemplateLiteralFinite(types2) {
  const result = FromTypes(types2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/template_literal/create.mjs
function TemplateLiteralCreate(pattern3) {
  return exports_memory.Create({ ["~kind"]: "TemplateLiteral" }, { type: "string", pattern: pattern3 }, {});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/template_literal/decode.mjs
function FromLiteralPush(variants, value, result = []) {
  return exports_guard.TakeLeft(variants, (left, right) => FromLiteralPush(right, value, [...result, `${left}${value}`]), () => result);
}
function FromLiteral2(variants, value) {
  return exports_guard.IsEqual(variants.length, 0) ? [`${value}`] : FromLiteralPush(variants, value);
}
function FromUnion(variants, types2, result = []) {
  return exports_guard.TakeLeft(types2, (left, right) => FromUnion(variants, right, [...result, ...FromType2(variants, left)]), () => result);
}
function FromType2(variants, type3) {
  const result = IsUnion(type3) ? FromUnion(variants, type3.anyOf) : IsLiteral(type3) ? FromLiteral2(variants, type3.const) : Unreachable();
  return result;
}
function DecodeFromSpan(variants, types2) {
  return exports_guard.TakeLeft(types2, (left, right) => DecodeFromSpan(FromType2(variants, left), right), () => variants);
}
function VariantsToLiterals(variants) {
  return variants.map((variant) => Literal(variant));
}
function DecodeTypesAsUnion(types2) {
  const variants = DecodeFromSpan([], types2);
  const literals = VariantsToLiterals(variants);
  const result = Union(literals);
  return result;
}
function DecodeTypes(types2) {
  return exports_guard.IsEqual(types2.length, 0) ? Unreachable() : exports_guard.IsEqual(types2.length, 1) && IsLiteral(types2[0]) ? types2[0] : DecodeTypesAsUnion(types2);
}
function TemplateLiteralDecodeUnsafe(pattern3) {
  const types2 = ParsePatternIntoTypes(pattern3);
  const result = exports_guard.IsEqual(types2.length, 0) ? String2() : IsTemplateLiteralFinite(types2) ? DecodeTypes(types2) : TemplateLiteralCreate(pattern3);
  return result;
}
function TemplateLiteralDecode(pattern3) {
  const decoded = TemplateLiteralDecodeUnsafe(pattern3);
  const result = IsTemplateLiteral(decoded) ? String2() : decoded;
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/record_create.mjs
function CreateRecord(key, value) {
  const type3 = "object";
  const patternProperties3 = { [key]: value };
  return exports_memory.Create({ ["~kind"]: "Record" }, { type: type3, patternProperties: patternProperties3 });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_any.mjs
function FromAnyKey(value) {
  return CreateRecord(StringKey, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_boolean.mjs
function FromBooleanKey(value) {
  return _Object_({ true: value, false: value });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/enum/enum_to_union.mjs
function FromEnumValue(value) {
  return exports_guard.IsString(value) || exports_guard.IsNumber(value) ? Literal(value) : exports_guard.IsNull(value) ? Null() : Never();
}
function EnumValuesToVariants(values) {
  const result = values.map((value) => FromEnumValue(value));
  return result;
}
function EnumValuesToUnion(values) {
  const variants = EnumValuesToVariants(values);
  const result = Union(variants);
  return result;
}
function EnumToUnion(type3) {
  const result = EnumValuesToUnion(type3.enum);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_enum.mjs
function FromEnumKey(values, value) {
  const unionKey = EnumValuesToUnion(values);
  const result = FromKey(unionKey, value);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_integer.mjs
function FromIntegerKey(_key, value) {
  const result = CreateRecord(IntegerKey, value);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/tuple.mjs
function Tuple(types2, options = {}) {
  const [items3, minItems3, additionalItems3] = [types2, types2.length, false];
  return exports_memory.Create({ ["~kind"]: "Tuple" }, { type: "array", additionalItems: additionalItems3, items: items3, minItems: minItems3 }, options);
}
function IsTuple(value) {
  return IsKind(value, "Tuple");
}
function TupleOptions(type3) {
  return exports_memory.Discard(type3, ["~kind", "type", "items", "minItems", "additionalItems"]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/tuple/to_object.mjs
function TupleElementsToProperties(types2) {
  const result = types2.reduceRight((result2, right, index2) => {
    return { [index2]: right, ...result2 };
  }, {});
  return result;
}
function TupleToObject(type3) {
  const properties3 = TupleElementsToProperties(type3.items);
  const result = _Object_(properties3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/composite.mjs
function IsReadonlyProperty(left, right) {
  return IsReadonly(left) ? IsReadonly(right) ? true : false : false;
}
function IsOptionalProperty(left, right) {
  return IsOptional(left) ? IsOptional(right) ? true : false : false;
}
function CompositeProperty(left, right) {
  const isReadonly = IsReadonlyProperty(left, right);
  const isOptional = IsOptionalProperty(left, right);
  const evaluated = EvaluateIntersect([left, right]);
  const property = ReadonlyRemove(OptionalRemove(evaluated));
  return isReadonly && isOptional ? ReadonlyAdd(OptionalAdd(property)) : isReadonly && !isOptional ? ReadonlyAdd(property) : !isReadonly && isOptional ? OptionalAdd(property) : property;
}
function CompositePropertyKey(left, right, key) {
  return key in left ? key in right ? CompositeProperty(left[key], right[key]) : left[key] : (key in right) ? right[key] : Never();
}
function CompositeProperties(left, right) {
  const keys = new Set([...exports_guard.Keys(right), ...exports_guard.Keys(left)]);
  return [...keys].reduce((result, key) => {
    return { ...result, [key]: CompositePropertyKey(left, right, key) };
  }, {});
}
function GetProperties(type3) {
  const result = IsObject3(type3) ? type3.properties : IsTuple(type3) ? TupleElementsToProperties(type3.items) : Unreachable();
  return result;
}
function Composite(left, right) {
  const leftProperties = GetProperties(left);
  const rightProperties = GetProperties(right);
  const properties3 = CompositeProperties(leftProperties, rightProperties);
  return _Object_(properties3);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/narrow.mjs
function Narrow(left, right) {
  const result = Compare(left, right);
  return exports_guard.IsEqual(result, ResultLeftInside) ? left : exports_guard.IsEqual(result, ResultRightInside) ? right : exports_guard.IsEqual(result, ResultEqual) ? right : Never();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/distribute.mjs
function IsObjectLike(type3) {
  return IsObject3(type3) || IsTuple(type3);
}
function IsUnionOperand(left, right) {
  const isUnionLeft = IsUnion(left);
  const isUnionRight = IsUnion(right);
  const result = isUnionLeft || isUnionRight;
  return result;
}
function DistributeOperation(left, right) {
  const evaluatedLeft = EvaluateType(left);
  const evaluatedRight = EvaluateType(right);
  const isUnionOperand = IsUnionOperand(evaluatedLeft, evaluatedRight);
  const isObjectLeft = IsObjectLike(evaluatedLeft);
  const IsObjectRight = IsObjectLike(evaluatedRight);
  const result = isUnionOperand ? EvaluateIntersect([evaluatedLeft, evaluatedRight]) : isObjectLeft && IsObjectRight ? Composite(evaluatedLeft, evaluatedRight) : isObjectLeft && !IsObjectRight ? evaluatedLeft : !isObjectLeft && IsObjectRight ? evaluatedRight : Narrow(evaluatedLeft, evaluatedRight);
  return result;
}
function DistributeType(type3, types2, result = []) {
  return exports_guard.TakeLeft(types2, (left, right) => DistributeType(type3, right, [...result, DistributeOperation(type3, left)]), () => exports_guard.IsEqual(result.length, 0) ? [type3] : result);
}
function DistributeUnion(types2, distribution, result = []) {
  return exports_guard.TakeLeft(types2, (left, right) => DistributeUnion(right, distribution, [...result, ...Distribute([left], distribution)]), () => result);
}
function Distribute(types2, result = []) {
  return exports_guard.TakeLeft(types2, (left, right) => IsUnion(left) ? Distribute(right, DistributeUnion(left.anyOf, result)) : Distribute(right, DistributeType(left, result)), () => result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/evaluate.mjs
function EvaluateIntersect(types2) {
  const distribution = Distribute(types2);
  const result = Broaden(distribution);
  return result;
}
function EvaluateUnion(types2) {
  const result = Broaden(types2);
  return result;
}
function EvaluateType(type3) {
  return IsIntersect(type3) ? EvaluateIntersect(type3.allOf) : IsUnion(type3) ? EvaluateUnion(type3.anyOf) : type3;
}
function EvaluateUnionFast(types2) {
  const result = exports_guard.IsEqual(types2.length, 1) ? types2[0] : exports_guard.IsEqual(types2.length, 0) ? Never() : Union(types2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_intersect.mjs
function FromIntersectKey(types2, value) {
  const evaluatedKey = EvaluateIntersect(types2);
  const result = FromKey(evaluatedKey, value);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_literal.mjs
function FromLiteralKey(key, value) {
  return exports_guard.IsString(key) || exports_guard.IsNumber(key) ? _Object_({ [key]: value }) : exports_guard.IsEqual(key, false) ? _Object_({ false: value }) : exports_guard.IsEqual(key, true) ? _Object_({ true: value }) : _Object_({});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_number.mjs
function FromNumberKey(_key, value) {
  const result = CreateRecord(NumberKey, value);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_string.mjs
function FromStringKey(key, value) {
  return exports_guard.HasPropertyKey(key, "pattern") && (exports_guard.IsString(key.pattern) || key.pattern instanceof RegExp) ? CreateRecord(key.pattern.toString(), value) : CreateRecord(StringKey, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_template_literal.mjs
function FromTemplateKey(pattern3, value) {
  const types2 = ParsePatternIntoTypes(pattern3);
  const finite = IsTemplateLiteralFinite(types2);
  const result = finite ? FromKey(TemplateLiteralDecode(pattern3), value) : CreateRecord(pattern3, value);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/flatten.mjs
function FlattenType(type3) {
  const result = IsUnion(type3) ? Flatten(type3.anyOf) : [type3];
  return result;
}
function Flatten(types2) {
  return types2.reduce((result, type3) => {
    return [...result, ...FlattenType(type3)];
  }, []);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key_union.mjs
function StringOrNumberCheck(types2) {
  return types2.some((type3) => IsString4(type3) || IsNumber4(type3) || IsInteger3(type3));
}
function TryBuildRecord(types2, value) {
  return exports_guard.IsEqual(StringOrNumberCheck(types2), true) ? CreateRecord(StringKey, value) : undefined;
}
function CreateProperties(types2, value) {
  return types2.reduce((result, left) => {
    return IsLiteral(left) && (exports_guard.IsString(left.const) || exports_guard.IsNumber(left.const)) ? { ...result, [left.const]: value } : result;
  }, {});
}
function CreateObject(types2, value) {
  const properties3 = CreateProperties(types2, value);
  const result = _Object_(properties3);
  return result;
}
function FromUnionKey(types2, value) {
  const flattened = Flatten(types2);
  const record = TryBuildRecord(flattened, value);
  return IsSchema2(record) ? record : CreateObject(flattened, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/from_key.mjs
function FromKey(key, value) {
  const result = IsAny(key) ? FromAnyKey(value) : IsBoolean4(key) ? FromBooleanKey(value) : IsEnum2(key) ? FromEnumKey(key.enum, value) : IsInteger3(key) ? FromIntegerKey(key, value) : IsIntersect(key) ? FromIntersectKey(key.allOf, value) : IsLiteral(key) ? FromLiteralKey(key.const, value) : IsNumber4(key) ? FromNumberKey(key, value) : IsUnion(key) ? FromUnionKey(key.anyOf, value) : IsString4(key) ? FromStringKey(key, value) : IsTemplateLiteral(key) ? FromTemplateKey(key.pattern, value) : _Object_({});
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/record/instantiate.mjs
function RecordAction(key, value, options) {
  const result = CanInstantiate([key]) ? exports_memory.Update(FromKey(key, value), {}, options) : RecordDeferred(key, value, options);
  return result;
}
function RecordInstantiate(context, state2, key, value, options) {
  const instantiatedKey = InstantiateType(context, state2, key);
  const instantiatedValue = InstantiateType(context, state2, value);
  return RecordAction(instantiatedKey, instantiatedValue, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/record.mjs
var IntegerKey = `^${IntegerPattern}$`;
var NumberKey = `^${NumberPattern}$`;
var StringKey = `^${StringPattern}$`;
function RecordDeferred(key, value, options = {}) {
  return Deferred("Record", [key, value], options);
}
function Record(key, value, options = {}) {
  return RecordAction(key, value, options);
}
function RecordFromPattern(key, value) {
  return CreateRecord(key, value);
}
function RecordPattern(type3) {
  return exports_guard.Keys(type3.patternProperties)[0];
}
function RecordKey(type3) {
  const pattern3 = RecordPattern(type3);
  const result = exports_guard.IsEqual(pattern3, StringKey) ? String2() : exports_guard.IsEqual(pattern3, IntegerKey) ? Integer() : exports_guard.IsEqual(pattern3, NumberKey) ? Number2() : TemplateLiteralDecodeUnsafe(pattern3);
  return result;
}
function RecordValue(type3) {
  return type3.patternProperties[RecordPattern(type3)];
}
function IsRecord(value) {
  return IsKind(value, "Record");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/rest.mjs
function Rest(type3) {
  return exports_memory.Create({ "~kind": "Rest" }, { type: "rest", items: type3 }, {});
}
function IsRest(value) {
  return IsKind(value, "Rest");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/this.mjs
function IsThis(value) {
  return IsKind(value, "This");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/undefined.mjs
function Undefined(options) {
  return exports_memory.Create({ "~kind": "Undefined" }, { type: "undefined" }, options);
}
function IsUndefined3(value) {
  return IsKind(value, "Undefined");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/void.mjs
function IsVoid(value) {
  return IsKind(value, "Void");
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/mapping.mjs
function PatternBigIntMapping(input) {
  return BigInt2();
}
function PatternStringMapping(input) {
  return String2();
}
function PatternNumberMapping(input) {
  return Number2();
}
function PatternIntegerMapping(input) {
  return Integer();
}
function PatternNeverMapping(input) {
  return Never();
}
function PatternTextMapping(input) {
  return Literal(input);
}
function PatternBaseMapping(input) {
  return input;
}
function PatternGroupMapping(input) {
  return Union(input[1]);
}
function PatternUnionMapping(input) {
  return input.length === 3 ? [...input[0], ...input[2]] : input.length === 1 ? [...input[0]] : [];
}
function PatternTermMapping(input) {
  return [input[0], ...input[1]];
}
function PatternBodyMapping(input) {
  return input;
}
function PatternMapping(input) {
  return input[1];
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/internal/match.mjs
function IsMatch(value) {
  return IsEqual(value.length, 2);
}
function Match3(input, ok, fail) {
  return IsMatch(input) ? ok(input[0], input[1]) : fail();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/internal/take.mjs
function TakeVariant(variant, input) {
  return IsEqual(input.indexOf(variant), 0) ? [variant, input.slice(variant.length)] : [];
}
function Take(variants, input) {
  for (let i = 0;i < variants.length; i++) {
    const result = TakeVariant(variants[i], input);
    if (IsMatch(result))
      return result;
  }
  return [];
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/internal/char.mjs
function Range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, i) => String.fromCharCode(start + i));
}
var Alpha = [
  ...Range(97, 122),
  ...Range(65, 90)
];
var Zero = "0";
var NonZero = Range(49, 57);
var Digit = [Zero, ...NonZero];
var WhiteSpace = " ";
var NewLine = `
`;
var UnderScore = "_";
var DollarSign = "$";

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/internal/trim.mjs
var LineComment = "//";
var OpenComment = "/*";
var CloseComment = "*/";
function DiscardMultilineComment(input) {
  const index2 = input.indexOf(CloseComment);
  const result = IsEqual(index2, -1) ? "" : input.slice(index2 + 2);
  return result;
}
function DiscardLineComment(input) {
  const index2 = input.indexOf(NewLine);
  const result = IsEqual(index2, -1) ? "" : input.slice(index2);
  return result;
}
function TrimStartUntilNewline(input) {
  return input.replace(/^[ \t\r\f\v]+/, "");
}
function TrimWhitespace(input) {
  const trimmed = TrimStartUntilNewline(input);
  return trimmed.startsWith(OpenComment) ? TrimWhitespace(DiscardMultilineComment(trimmed.slice(2))) : trimmed.startsWith(LineComment) ? TrimWhitespace(DiscardLineComment(trimmed.slice(2))) : trimmed;
}
function Trim(input) {
  const trimmed = input.trimStart();
  return trimmed.startsWith(OpenComment) ? Trim(DiscardMultilineComment(trimmed.slice(2))) : trimmed.startsWith(LineComment) ? Trim(DiscardLineComment(trimmed.slice(2))) : trimmed;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/unsigned_integer.mjs
var AllowedDigits = [...Digit, UnderScore];
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/const.mjs
function TakeConst(const_, input) {
  return Take([const_], input);
}
function Const(const_, input) {
  return IsEqual(const_, "") ? ["", input] : const_.startsWith(NewLine) ? TakeConst(const_, TrimWhitespace(input)) : const_.startsWith(WhiteSpace) ? TakeConst(const_, input) : TakeConst(const_, Trim(input));
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/ident.mjs
var Initial = [...Alpha, UnderScore, DollarSign];
var Remaining = [...Initial, ...Digit];
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/unsigned_number.mjs
var AllowedDigits2 = [...Digit, UnderScore];
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/until.mjs
function TakeOne(input) {
  const result = IsEqual(input, "") ? [] : [input.slice(0, 1), input.slice(1)];
  return result;
}
function IsInputMatchSentinal(end, input) {
  return TakeLeft(end, (left, right) => input.startsWith(left) ? true : IsInputMatchSentinal(right, input), () => false);
}
function Until(end, input, result = "") {
  return Match3(TakeOne(input), (One, Rest2) => IsInputMatchSentinal(end, input) ? [result, input] : Until(end, Rest2, `${result}${One}`), () => []);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/token/until_1.mjs
function Until_1(end, input) {
  return Match3(Until(end, input), (Until2, UntilRest) => IsEqual(Until2, "") ? [] : [Until2, UntilRest], () => []);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/script/parser.mjs
var If2 = (result, left, right = () => []) => result.length === 2 ? left(result) : right();
var PatternBigInt = (input) => If2(Const("-?(?:0|[1-9][0-9]*)n", input), ([_0, input2]) => [PatternBigIntMapping(_0), input2]);
var PatternString = (input) => If2(Const(".*", input), ([_0, input2]) => [PatternStringMapping(_0), input2]);
var PatternNumber = (input) => If2(Const("-?(?:0|[1-9][0-9]*)(?:.[0-9]+)?", input), ([_0, input2]) => [PatternNumberMapping(_0), input2]);
var PatternInteger = (input) => If2(Const("-?(?:0|[1-9][0-9]*)", input), ([_0, input2]) => [PatternIntegerMapping(_0), input2]);
var PatternNever = (input) => If2(Const("(?!)", input), ([_0, input2]) => [PatternNeverMapping(_0), input2]);
var PatternText = (input) => If2(Until_1(["-?(?:0|[1-9][0-9]*)n", ".*", "-?(?:0|[1-9][0-9]*)(?:.[0-9]+)?", "-?(?:0|[1-9][0-9]*)", "(?!)", "(", ")", "$", "|"], input), ([_0, input2]) => [PatternTextMapping(_0), input2]);
var PatternBase = (input) => If2(If2(PatternBigInt(input), ([_0, input2]) => [_0, input2], () => If2(PatternString(input), ([_0, input2]) => [_0, input2], () => If2(PatternNumber(input), ([_0, input2]) => [_0, input2], () => If2(PatternInteger(input), ([_0, input2]) => [_0, input2], () => If2(PatternNever(input), ([_0, input2]) => [_0, input2], () => If2(PatternGroup(input), ([_0, input2]) => [_0, input2], () => If2(PatternText(input), ([_0, input2]) => [_0, input2], () => []))))))), ([_0, input2]) => [PatternBaseMapping(_0), input2]);
var PatternGroup = (input) => If2(If2(Const("(", input), ([_0, input2]) => If2(PatternBody(input2), ([_1, input3]) => If2(Const(")", input3), ([_2, input4]) => [[_0, _1, _2], input4]))), ([_0, input2]) => [PatternGroupMapping(_0), input2]);
var PatternUnion = (input) => If2(If2(If2(PatternTerm(input), ([_0, input2]) => If2(Const("|", input2), ([_1, input3]) => If2(PatternUnion(input3), ([_2, input4]) => [[_0, _1, _2], input4]))), ([_0, input2]) => [_0, input2], () => If2(If2(PatternTerm(input), ([_0, input2]) => [[_0], input2]), ([_0, input2]) => [_0, input2], () => If2([[], input], ([_0, input2]) => [_0, input2], () => []))), ([_0, input2]) => [PatternUnionMapping(_0), input2]);
var PatternTerm = (input) => If2(If2(PatternBase(input), ([_0, input2]) => If2(PatternBody(input2), ([_1, input3]) => [[_0, _1], input3])), ([_0, input2]) => [PatternTermMapping(_0), input2]);
var PatternBody = (input) => If2(If2(PatternUnion(input), ([_0, input2]) => [_0, input2], () => If2(PatternTerm(input), ([_0, input2]) => [_0, input2], () => [])), ([_0, input2]) => [PatternBodyMapping(_0), input2]);
var Pattern = (input) => If2(If2(Const("^", input), ([_0, input2]) => If2(PatternBody(input2), ([_1, input3]) => If2(Const("$", input3), ([_2, input4]) => [[_0, _1, _2], input4]))), ([_0, input2]) => [PatternMapping(_0), input2]);

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/template_literal/encode.mjs
function JoinString(input) {
  return input.join("|");
}
function UnwrapTemplateLiteralPattern(pattern3) {
  return pattern3.slice(1, pattern3.length - 1);
}
function EncodeLiteral(value, right, pattern3) {
  return EncodeTypes(right, `${pattern3}${value}`);
}
function EncodeBigInt(right, pattern3) {
  return EncodeTypes(right, `${pattern3}${BigIntPattern}`);
}
function EncodeInteger(right, pattern3) {
  return EncodeTypes(right, `${pattern3}${IntegerPattern}`);
}
function EncodeNumber(right, pattern3) {
  return EncodeTypes(right, `${pattern3}${NumberPattern}`);
}
function EncodeBoolean(right, pattern3) {
  return EncodeType(Union([Literal("false"), Literal("true")]), right, pattern3);
}
function EncodeString(right, pattern3) {
  return EncodeTypes(right, `${pattern3}${StringPattern}`);
}
function EncodeTemplateLiteral(templatePattern, right, pattern3) {
  return EncodeTypes(right, `${pattern3}${UnwrapTemplateLiteralPattern(templatePattern)}`);
}
function EncodeTemplateLiteralDeferred(types2, right, pattern3) {
  const templateLiteral = TemplateLiteralAction(types2, {});
  const result = EncodeType(templateLiteral, right, pattern3);
  return result;
}
function EncodeEnum(types2, right, pattern3) {
  const variants = EnumValuesToVariants(types2);
  return EncodeUnion(variants, right, pattern3);
}
function EncodeUnion(types2, right, pattern3, result = []) {
  return exports_guard.TakeLeft(types2, (head, tail) => EncodeUnion(tail, right, pattern3, [...result, EncodeType(head, [], "")]), () => EncodeTypes(right, `${pattern3}(${JoinString(result)})`));
}
function EncodeType(type3, right, pattern3) {
  return IsEnum2(type3) ? EncodeEnum(type3.enum, right, pattern3) : IsInteger3(type3) ? EncodeInteger(right, pattern3) : IsLiteral(type3) ? EncodeLiteral(type3.const, right, pattern3) : IsBigInt3(type3) ? EncodeBigInt(right, pattern3) : IsBoolean4(type3) ? EncodeBoolean(right, pattern3) : IsNumber4(type3) ? EncodeNumber(right, pattern3) : IsString4(type3) ? EncodeString(right, pattern3) : IsTemplateLiteral(type3) ? EncodeTemplateLiteral(type3.pattern, right, pattern3) : IsTemplateLiteralDeferred(type3) ? EncodeTemplateLiteralDeferred(type3.parameters[0], right, pattern3) : IsUnion(type3) ? EncodeUnion(type3.anyOf, right, pattern3) : NeverPattern;
}
function EncodeTypes(types2, pattern3) {
  return exports_guard.TakeLeft(types2, (left, right) => EncodeType(left, right, pattern3), () => pattern3);
}
function EncodePattern(types2) {
  const encoded = EncodeTypes(types2, "");
  const result = `^${encoded}$`;
  return result;
}
function TemplateLiteralEncode(types2) {
  const pattern3 = EncodePattern(types2);
  const result = TemplateLiteralCreate(pattern3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/template_literal/instantiate.mjs
function TemplateLiteralAction(types2, options) {
  const result = CanInstantiate(types2) ? exports_memory.Update(TemplateLiteralEncode(types2), {}, options) : TemplateLiteralDeferred(types2, options);
  return result;
}
function TemplateLiteralInstantiate(context, state2, types2, options) {
  const instantiatedTypes = InstantiateTypes(context, state2, types2);
  return TemplateLiteralAction(instantiatedTypes, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/template_literal.mjs
function TemplateLiteralDeferred(types2, options = {}) {
  return Deferred("TemplateLiteral", [types2], options);
}
function IsTemplateLiteralDeferred(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "action") && exports_guard.IsEqual(value.action, "TemplateLiteral");
}
function IsTemplateLiteral(value) {
  return IsKind(value, "TemplateLiteral");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/result.mjs
var exports_result = {};
__export(exports_result, {
  Match: () => Match4,
  IsExtendsUnion: () => IsExtendsUnion,
  IsExtendsTrueLike: () => IsExtendsTrueLike,
  IsExtendsTrue: () => IsExtendsTrue,
  IsExtendsFalse: () => IsExtendsFalse,
  ExtendsUnion: () => ExtendsUnion,
  ExtendsTrue: () => ExtendsTrue,
  ExtendsFalse: () => ExtendsFalse
});
function ExtendsUnion(inferred) {
  return exports_memory.Create({ ["~kind"]: "ExtendsUnion" }, { inferred });
}
function IsExtendsUnion(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "inferred") && exports_guard.IsEqual(value["~kind"], "ExtendsUnion") && exports_guard.IsObject(value.inferred);
}
function ExtendsTrue(inferred) {
  return exports_memory.Create({ ["~kind"]: "ExtendsTrue" }, { inferred });
}
function IsExtendsTrue(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "inferred") && exports_guard.IsEqual(value["~kind"], "ExtendsTrue") && exports_guard.IsObject(value.inferred);
}
function ExtendsFalse() {
  return exports_memory.Create({ ["~kind"]: "ExtendsFalse" }, {});
}
function IsExtendsFalse(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.IsEqual(value["~kind"], "ExtendsFalse");
}
function IsExtendsTrueLike(value) {
  return IsExtendsUnion(value) || IsExtendsTrue(value);
}
function Match4(result, true_, false_) {
  return IsExtendsTrueLike(result) ? true_(result.inferred) : false_();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/extends_right.mjs
function ExtendsRightInfer(inferred, name, left, right) {
  return Match4(ExtendsLeft(inferred, left, right), (checkInferred) => ExtendsTrue(exports_memory.Assign(exports_memory.Assign(inferred, checkInferred), { [name]: left })), () => ExtendsFalse());
}
function ExtendsRightAny(inferred, _left) {
  return ExtendsTrue(inferred);
}
function ExtendsRightEnum(inferred, left, right) {
  const union2 = EnumValuesToUnion(right);
  return ExtendsLeft(inferred, left, union2);
}
function ExtendsRightIntersect(inferred, left, right) {
  return exports_guard.TakeLeft(right, (head, tail) => Match4(ExtendsLeft(inferred, left, head), (inferred2) => ExtendsRightIntersect(inferred2, left, tail), () => ExtendsFalse()), () => ExtendsTrue(inferred));
}
function ExtendsRightTemplateLiteral(inferred, left, right) {
  const decoded = TemplateLiteralDecode(right);
  return ExtendsLeft(inferred, left, decoded);
}
function ExtendsRightUnion(inferred, left, right) {
  return exports_guard.TakeLeft(right, (head, tail) => Match4(ExtendsLeft(inferred, left, head), (inferred2) => ExtendsTrue(inferred2), () => ExtendsRightUnion(inferred, left, tail)), () => ExtendsFalse());
}
function ExtendsRight(inferred, left, right) {
  return IsAny(right) ? ExtendsRightAny(inferred, left) : IsEnum2(right) ? ExtendsRightEnum(inferred, left, right.enum) : IsInfer(right) ? ExtendsRightInfer(inferred, right.name, left, right.extends) : IsIntersect(right) ? ExtendsRightIntersect(inferred, left, right.allOf) : IsTemplateLiteral(right) ? ExtendsRightTemplateLiteral(inferred, left, right.pattern) : IsUnion(right) ? ExtendsRightUnion(inferred, left, right.anyOf) : IsUnknown(right) ? ExtendsTrue(inferred) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/any.mjs
function ExtendsAny(inferred, left, right) {
  return IsInfer(right) ? ExtendsRight(inferred, left, right) : IsAny(right) ? ExtendsTrue(inferred) : IsUnknown(right) ? ExtendsTrue(inferred) : ExtendsUnion(inferred);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/array.mjs
function ExtendsImmutable(left, right) {
  const isImmutableLeft = IsImmutable(left);
  const isImmutableRight = IsImmutable(right);
  return isImmutableLeft && isImmutableRight ? true : !isImmutableLeft && isImmutableRight ? true : isImmutableLeft && !isImmutableRight ? false : true;
}
function ExtendsArray(inferred, arrayLeft, left, right) {
  return IsArray3(right) ? ExtendsImmutable(arrayLeft, right) ? ExtendsLeft(inferred, left, right.items) : ExtendsFalse() : ExtendsRight(inferred, arrayLeft, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/async_iterator.mjs
function ExtendsAsyncIterator(inferred, left, right) {
  return IsAsyncIterator3(right) ? ExtendsLeft(inferred, left, right.iteratorItems) : ExtendsRight(inferred, AsyncIterator(left), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/bigint.mjs
function ExtendsBigInt(inferred, left, right) {
  return IsBigInt3(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/boolean.mjs
function ExtendsBoolean(inferred, left, right) {
  return IsBoolean4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/parameters.mjs
function ParameterCompare(inferred, left, leftRest, right, rightRest) {
  const checkLeft = IsInfer(right) ? left : right;
  const checkRight = IsInfer(right) ? right : left;
  const isLeftOptional = IsOptional(left);
  const isRightOptional = IsOptional(right);
  return !isLeftOptional && isRightOptional ? ExtendsFalse() : Match4(ExtendsLeft(inferred, checkLeft, checkRight), (inferred2) => ExtendsParameters(inferred2, leftRest, rightRest), () => ExtendsFalse());
}
function ParameterRight(inferred, left, leftRest, rightRest) {
  return exports_guard.TakeLeft(rightRest, (head, tail) => ParameterCompare(inferred, left, leftRest, head, tail), () => IsOptional(left) ? ExtendsTrue(inferred) : ExtendsFalse());
}
function ParametersLeft(inferred, left, rightRest) {
  return exports_guard.TakeLeft(left, (head, tail) => ParameterRight(inferred, head, tail, rightRest), () => ExtendsTrue(inferred));
}
function ExtendsParameters(inferred, left, right) {
  return ParametersLeft(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/return_type.mjs
function ExtendsReturnType(inferred, left, right) {
  return IsVoid(right) ? ExtendsTrue(inferred) : ExtendsLeft(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/constructor.mjs
function ExtendsConstructor(inferred, parameters, returnType, right) {
  return IsAny(right) ? ExtendsTrue(inferred) : IsUnknown(right) ? ExtendsTrue(inferred) : IsConstructor3(right) ? Match4(ExtendsParameters(inferred, parameters, right["parameters"]), (inferred2) => ExtendsReturnType(inferred2, returnType, right["instanceType"]), () => ExtendsFalse()) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/enum.mjs
function ExtendsEnum(inferred, left, right) {
  return ExtendsLeft(inferred, EnumToUnion(left), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/function.mjs
function ExtendsFunction(inferred, parameters, returnType, right) {
  return IsAny(right) ? ExtendsTrue(inferred) : IsUnknown(right) ? ExtendsTrue(inferred) : IsFunction3(right) ? Match4(ExtendsParameters(inferred, parameters, right["parameters"]), (inferred2) => ExtendsReturnType(inferred2, returnType, right["returnType"]), () => ExtendsFalse()) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/integer.mjs
function ExtendsInteger(inferred, left, right) {
  return IsInteger3(right) ? ExtendsTrue(inferred) : IsNumber4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/intersect.mjs
function ExtendsIntersect(inferred, left, right) {
  const evaluated = EvaluateIntersect(left);
  return ExtendsLeft(inferred, evaluated, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/iterator.mjs
function ExtendsIterator(inferred, left, right) {
  return IsIterator3(right) ? ExtendsLeft(inferred, left, right.iteratorItems) : ExtendsRight(inferred, Iterator(left), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/literal.mjs
function ExtendsLiteralValue(inferred, left, right) {
  return left === right ? ExtendsTrue(inferred) : ExtendsFalse();
}
function ExtendsLiteralBigInt(inferred, left, right) {
  return IsLiteral(right) ? ExtendsLiteralValue(inferred, left, right.const) : IsBigInt3(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, Literal(left), right);
}
function ExtendsLiteralBoolean(inferred, left, right) {
  return IsLiteral(right) ? ExtendsLiteralValue(inferred, left, right.const) : IsBoolean4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, Literal(left), right);
}
function ExtendsLiteralNumber(inferred, left, right) {
  return IsLiteral(right) ? ExtendsLiteralValue(inferred, left, right.const) : IsNumber4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, Literal(left), right);
}
function ExtendsLiteralString(inferred, left, right) {
  return IsLiteral(right) ? ExtendsLiteralValue(inferred, left, right.const) : IsString4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, Literal(left), right);
}
function ExtendsLiteral(inferred, left, right) {
  return exports_guard.IsBigInt(left.const) ? ExtendsLiteralBigInt(inferred, left.const, right) : exports_guard.IsBoolean(left.const) ? ExtendsLiteralBoolean(inferred, left.const, right) : exports_guard.IsNumber(left.const) ? ExtendsLiteralNumber(inferred, left.const, right) : exports_guard.IsString(left.const) ? ExtendsLiteralString(inferred, left.const, right) : Unreachable();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/never.mjs
function ExtendsNever(inferred, left, right) {
  return IsInfer(right) ? ExtendsRight(inferred, left, right) : ExtendsTrue(inferred);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/null.mjs
function ExtendsNull(inferred, left, right) {
  return IsNull3(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/number.mjs
function ExtendsNumber(inferred, left, right) {
  return IsNumber4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/object.mjs
function ExtendsPropertyOptional(inferred, left, right) {
  return IsOptional(left) ? IsOptional(right) ? ExtendsTrue(inferred) : ExtendsFalse() : ExtendsTrue(inferred);
}
function ExtendsProperty(inferred, left, right) {
  return IsInfer(right) && IsNever(right.extends) ? ExtendsFalse() : Match4(ExtendsLeft(inferred, left, right), (inferred2) => ExtendsPropertyOptional(inferred2, left, right), () => ExtendsFalse());
}
function ExtractInferredProperties(keys, properties4) {
  return keys.reduce((result, key) => {
    return key in properties4 ? IsExtendsTrueLike(properties4[key]) ? { ...result, ...properties4[key].inferred } : Unreachable() : Unreachable();
  }, {});
}
function ExtendsPropertiesComparer(inferred, left, right) {
  const properties4 = {};
  for (const rightKey of exports_guard.Keys(right)) {
    properties4[rightKey] = rightKey in left ? ExtendsProperty({}, left[rightKey], right[rightKey]) : IsOptional(right[rightKey]) ? IsInfer(right[rightKey]) ? ExtendsTrue(exports_memory.Assign(inferred, { [right[rightKey].name]: right[rightKey].extends })) : ExtendsTrue(inferred) : ExtendsFalse();
  }
  const checked = exports_guard.Values(properties4).every((result) => IsExtendsTrueLike(result));
  const extracted = checked ? ExtractInferredProperties(exports_guard.Keys(properties4), properties4) : {};
  return checked ? ExtendsTrue(extracted) : ExtendsFalse();
}
function ExtendsProperties(inferred, left, right) {
  const compared = ExtendsPropertiesComparer(inferred, left, right);
  return IsExtendsTrueLike(compared) ? ExtendsTrue(exports_memory.Assign(inferred, compared.inferred)) : ExtendsFalse();
}
function ExtendsObjectToObject(inferred, left, right) {
  return ExtendsProperties(inferred, left, right);
}
function ExtendsObject(inferred, left, right) {
  return IsObject3(right) ? ExtendsObjectToObject(inferred, left, right.properties) : ExtendsRight(inferred, _Object_(left), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/promise.mjs
function ExtendsPromise(inferred, left, right) {
  return IsPromise(right) ? ExtendsLeft(inferred, left, right.item) : ExtendsRight(inferred, _Promise_(left), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/string.mjs
function ExtendsString(inferred, left, right) {
  return IsString4(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/symbol.mjs
function ExtendsSymbol(inferred, left, right) {
  return IsSymbol3(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/template_literal.mjs
function ExtendsTemplateLiteral(inferred, left, right) {
  const decoded = TemplateLiteralDecode(left);
  return ExtendsLeft(inferred, decoded, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/inference.mjs
function Inferrable(name, type3) {
  return exports_memory.Create({ "~kind": "Inferrable" }, { name, type: type3 }, {});
}
function IsInferable(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "~kind") && exports_guard.HasPropertyKey(value, "name") && exports_guard.HasPropertyKey(value, "type") && exports_guard.IsEqual(value["~kind"], "Inferrable") && exports_guard.IsString(value.name) && exports_guard.IsObject(value.type);
}
function TryRestInferable(type3) {
  return IsRest(type3) ? IsInfer(type3.items) ? IsArray3(type3.items.extends) ? Inferrable(type3.items.name, type3.items.extends.items) : IsUnknown(type3.items.extends) ? Inferrable(type3.items.name, type3.items.extends) : undefined : Unreachable() : undefined;
}
function TryInferable(type3) {
  return IsInfer(type3) ? Inferrable(type3.name, type3.extends) : undefined;
}
function TryInferResults(rest3, right, result = []) {
  return exports_guard.TakeLeft(rest3, (head, tail) => Match4(ExtendsLeft({}, head, right), () => TryInferResults(tail, right, [...result, head]), () => {
    return;
  }), () => result);
}
function InferTupleResult(inferred, name, left, right) {
  const results = TryInferResults(left, right);
  return exports_guard.IsArray(results) ? ExtendsTrue(exports_memory.Assign(inferred, { [name]: Tuple(results) })) : ExtendsFalse();
}
function InferUnionResult(inferred, name, left, right) {
  const results = TryInferResults(left, right);
  return exports_guard.IsArray(results) ? ExtendsTrue(exports_memory.Assign(inferred, { [name]: Union(results) })) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/tuple.mjs
function Reverse(types2) {
  return [...types2].reverse();
}
function ApplyReverse(types2, reversed) {
  return reversed ? Reverse(types2) : types2;
}
function Reversed(types2) {
  const first = types2.length > 0 ? types2[0] : undefined;
  const inferrable = IsSchema2(first) ? TryRestInferable(first) : undefined;
  return IsSchema2(inferrable);
}
function ElementsCompare(inferred, reversed, left, leftRest, right, rightRest) {
  return Match4(ExtendsLeft(inferred, left, right), (checkInferred) => Elements(checkInferred, reversed, leftRest, rightRest), () => ExtendsFalse());
}
function ElementsLeft(inferred, reversed, leftRest, right, rightRest) {
  const inferable = TryRestInferable(right);
  return IsInferable(inferable) ? InferTupleResult(inferred, inferable["name"], ApplyReverse(leftRest, reversed), inferable["type"]) : exports_guard.TakeLeft(leftRest, (head, tail) => ElementsCompare(inferred, reversed, head, tail, right, rightRest), () => ExtendsFalse());
}
function ElementsRight(inferred, reversed, leftRest, rightRest) {
  return exports_guard.TakeLeft(rightRest, (head, tail) => ElementsLeft(inferred, reversed, leftRest, head, tail), () => exports_guard.IsEqual(leftRest.length, 0) ? ExtendsTrue(inferred) : ExtendsFalse());
}
function Elements(inferred, reversed, leftRest, rightRest) {
  return ElementsRight(inferred, reversed, leftRest, rightRest);
}
function ExtendsTupleToTuple(inferred, left, right) {
  const instantiatedRight = InstantiateElements(inferred, { callstack: [] }, right);
  const reversed = Reversed(instantiatedRight);
  return Elements(inferred, reversed, ApplyReverse(left, reversed), ApplyReverse(instantiatedRight, reversed));
}
function ExtendsTupleToArray(inferred, left, right) {
  const inferrable = TryInferable(right);
  return IsInferable(inferrable) ? InferUnionResult(inferred, inferrable["name"], left, inferrable["type"]) : exports_guard.TakeLeft(left, (head, tail) => Match4(ExtendsLeft(inferred, head, right), (inferred2) => ExtendsTupleToArray(inferred2, tail, right), () => ExtendsFalse()), () => ExtendsTrue(inferred));
}
function ExtendsTuple(inferred, left, right) {
  const instantiatedLeft = InstantiateElements(inferred, { callstack: [] }, left);
  return IsTuple(right) ? ExtendsTupleToTuple(inferred, instantiatedLeft, right.items) : IsArray3(right) ? ExtendsTupleToArray(inferred, instantiatedLeft, right.items) : ExtendsRight(inferred, Tuple(instantiatedLeft), right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/undefined.mjs
function ExtendsUndefined(inferred, left, right) {
  return IsVoid(right) ? ExtendsTrue(inferred) : IsUndefined3(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/union.mjs
function ExtendsUnionSome(inferred, type3, unionTypes) {
  return exports_guard.TakeLeft(unionTypes, (head, tail) => Match4(ExtendsLeft(inferred, type3, head), (inferred2) => ExtendsTrue(inferred2), () => ExtendsUnionSome(inferred, type3, tail)), () => ExtendsFalse());
}
function ExtendsUnionLeft(inferred, left, right) {
  return exports_guard.TakeLeft(left, (head, tail) => Match4(ExtendsUnionSome(inferred, head, right), (inferred2) => ExtendsUnionLeft(inferred2, tail, right), () => ExtendsFalse()), () => ExtendsTrue(inferred));
}
function ExtendsUnion2(inferred, left, right) {
  const inferrable = TryInferable(right);
  return IsInferable(inferrable) ? InferUnionResult(inferred, inferrable.name, left, inferrable.type) : IsUnion(right) ? ExtendsUnionLeft(inferred, left, right.anyOf) : ExtendsUnionLeft(inferred, left, [right]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/unknown.mjs
function ExtendsUnknown(inferred, left, right) {
  return IsInfer(right) ? ExtendsRight(inferred, left, right) : IsAny(right) ? ExtendsTrue(inferred) : IsUnknown(right) ? ExtendsTrue(inferred) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/void.mjs
function ExtendsVoid(inferred, left, right) {
  return IsVoid(right) ? ExtendsTrue(inferred) : ExtendsRight(inferred, left, right);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/extends_left.mjs
function ExtendsLeft(inferred, left, right) {
  return IsAny(left) ? ExtendsAny(inferred, left, right) : IsArray3(left) ? ExtendsArray(inferred, left, left.items, right) : IsAsyncIterator3(left) ? ExtendsAsyncIterator(inferred, left.iteratorItems, right) : IsBigInt3(left) ? ExtendsBigInt(inferred, left, right) : IsBoolean4(left) ? ExtendsBoolean(inferred, left, right) : IsConstructor3(left) ? ExtendsConstructor(inferred, left.parameters, left.instanceType, right) : IsEnum2(left) ? ExtendsEnum(inferred, left, right) : IsFunction3(left) ? ExtendsFunction(inferred, left.parameters, left.returnType, right) : IsInteger3(left) ? ExtendsInteger(inferred, left, right) : IsIntersect(left) ? ExtendsIntersect(inferred, left.allOf, right) : IsIterator3(left) ? ExtendsIterator(inferred, left.iteratorItems, right) : IsLiteral(left) ? ExtendsLiteral(inferred, left, right) : IsNever(left) ? ExtendsNever(inferred, left, right) : IsNull3(left) ? ExtendsNull(inferred, left, right) : IsNumber4(left) ? ExtendsNumber(inferred, left, right) : IsObject3(left) ? ExtendsObject(inferred, left.properties, right) : IsPromise(left) ? ExtendsPromise(inferred, left.item, right) : IsString4(left) ? ExtendsString(inferred, left, right) : IsSymbol3(left) ? ExtendsSymbol(inferred, left, right) : IsTemplateLiteral(left) ? ExtendsTemplateLiteral(inferred, left.pattern, right) : IsTuple(left) ? ExtendsTuple(inferred, left.items, right) : IsUndefined3(left) ? ExtendsUndefined(inferred, left, right) : IsUnion(left) ? ExtendsUnion2(inferred, left.anyOf, right) : IsUnknown(left) ? ExtendsUnknown(inferred, left, right) : IsVoid(left) ? ExtendsVoid(inferred, left, right) : ExtendsFalse();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/interface/instantiate.mjs
function InterfaceOperation(heritage, properties4) {
  const result = EvaluateIntersect([...heritage, _Object_(properties4)]);
  return result;
}
function InterfaceAction(heritage, properties4, options) {
  const result = CanInstantiate(heritage) ? exports_memory.Update(InterfaceOperation(heritage, properties4), {}, options) : InterfaceDeferred(heritage, properties4, options);
  return result;
}
function InterfaceInstantiate(context, state2, heritage, properties4, options) {
  const instantiatedHeritage = InstantiateTypes(context, state2, heritage);
  const instantiatedProperties = InstantiateProperties(context, state2, properties4);
  return InterfaceAction(instantiatedHeritage, instantiatedProperties, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/interface.mjs
function InterfaceDeferred(heritage, properties4, options = {}) {
  return Deferred("Interface", [heritage, properties4], options);
}
function IsInterfaceDeferred(value) {
  return IsSchema2(value) && exports_guard.HasPropertyKey(value, "action") && exports_guard.IsEqual(value.action, "Interface");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/check.mjs
function FromRef(stack, context, ref5) {
  return stack.includes(ref5) ? true : FromType3([...stack, ref5], context, context[ref5]);
}
function FromProperties(stack, context, properties4) {
  const types2 = PropertyValues(properties4);
  return FromTypes2(stack, context, types2);
}
function FromTypes2(stack, context, types2) {
  return exports_guard.TakeLeft(types2, (left, right) => FromType3(stack, context, left) ? true : FromTypes2(stack, context, right), () => false);
}
function FromType3(stack, context, type3) {
  return IsRef2(type3) ? FromRef(stack, context, type3.$ref) : IsArray3(type3) ? FromType3(stack, context, type3.items) : IsAsyncIterator3(type3) ? FromType3(stack, context, type3.iteratorItems) : IsConstructor3(type3) ? FromTypes2(stack, context, [...type3.parameters, type3.instanceType]) : IsFunction3(type3) ? FromTypes2(stack, context, [...type3.parameters, type3.returnType]) : IsInterfaceDeferred(type3) ? FromProperties(stack, context, type3.parameters[1]) : IsIntersect(type3) ? FromTypes2(stack, context, type3.allOf) : IsIterator3(type3) ? FromType3(stack, context, type3.iteratorItems) : IsObject3(type3) ? FromProperties(stack, context, type3.properties) : IsPromise(type3) ? FromType3(stack, context, type3.item) : IsUnion(type3) ? FromTypes2(stack, context, type3.anyOf) : IsTuple(type3) ? FromTypes2(stack, context, type3.items) : IsRecord(type3) ? FromType3(stack, context, RecordValue(type3)) : false;
}
function CyclicCheck(stack, context, type3) {
  const result = FromType3(stack, context, type3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/candidates.mjs
function ResolveCandidateKeys(context, keys) {
  return keys.reduce((result, left) => {
    return left in context ? CyclicCheck([left], context, context[left]) ? [...result, left] : result : Unreachable();
  }, []);
}
function CyclicCandidates(context) {
  const keys = PropertyKeys(context);
  const result = ResolveCandidateKeys(context, keys);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/dependencies.mjs
function FromRef2(context, ref5, result) {
  return result.includes(ref5) ? result : (ref5 in context) ? FromType4(context, context[ref5], [...result, ref5]) : Unreachable();
}
function FromProperties2(context, properties4, result) {
  const types2 = PropertyValues(properties4);
  return FromTypes3(context, types2, result);
}
function FromTypes3(context, types2, result) {
  return types2.reduce((result2, left) => {
    return FromType4(context, left, result2);
  }, result);
}
function FromType4(context, type3, result) {
  return IsRef2(type3) ? FromRef2(context, type3.$ref, result) : IsArray3(type3) ? FromType4(context, type3.items, result) : IsAsyncIterator3(type3) ? FromType4(context, type3.iteratorItems, result) : IsConstructor3(type3) ? FromTypes3(context, [...type3.parameters, type3.instanceType], result) : IsFunction3(type3) ? FromTypes3(context, [...type3.parameters, type3.returnType], result) : IsInterfaceDeferred(type3) ? FromProperties2(context, type3.parameters[1], result) : IsIntersect(type3) ? FromTypes3(context, type3.allOf, result) : IsIterator3(type3) ? FromType4(context, type3.iteratorItems, result) : IsObject3(type3) ? FromProperties2(context, type3.properties, result) : IsPromise(type3) ? FromType4(context, type3.item, result) : IsUnion(type3) ? FromTypes3(context, type3.anyOf, result) : IsTuple(type3) ? FromTypes3(context, type3.items, result) : IsRecord(type3) ? FromType4(context, RecordValue(type3), result) : result;
}
function CyclicDependencies(context, key, type3) {
  const result = FromType4(context, type3, [key]);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/extends.mjs
function FromRef3(_ref) {
  return Any();
}
function FromProperties3(properties4) {
  return exports_guard.Keys(properties4).reduce((result, key) => {
    return { ...result, [key]: FromType5(properties4[key]) };
  }, {});
}
function FromTypes4(types2) {
  return types2.reduce((result, left) => {
    return [...result, FromType5(left)];
  }, []);
}
function FromType5(type3) {
  return IsRef2(type3) ? FromRef3(type3.$ref) : IsArray3(type3) ? _Array_(FromType5(type3.items), ArrayOptions(type3)) : IsAsyncIterator3(type3) ? AsyncIterator(FromType5(type3.iteratorItems)) : IsConstructor3(type3) ? Constructor(FromTypes4(type3.parameters), FromType5(type3.instanceType)) : IsFunction3(type3) ? _Function_(FromTypes4(type3.parameters), FromType5(type3.returnType)) : IsIntersect(type3) ? Intersect(FromTypes4(type3.allOf)) : IsIterator3(type3) ? Iterator(FromType5(type3.iteratorItems)) : IsObject3(type3) ? _Object_(FromProperties3(type3.properties)) : IsPromise(type3) ? _Promise_(FromType5(type3.item)) : IsRecord(type3) ? Record(RecordKey(type3), FromType5(RecordValue(type3))) : IsUnion(type3) ? Union(FromTypes4(type3.anyOf)) : IsTuple(type3) ? Tuple(FromTypes4(type3.items)) : type3;
}
function CyclicAnyFromParameters(defs2, ref5) {
  return ref5 in defs2 ? FromType5(defs2[ref5]) : Unknown();
}
function CyclicExtends(type3) {
  return CyclicAnyFromParameters(type3.$defs, type3.$ref);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/instantiate.mjs
function CyclicInterface(context, heritage, properties4) {
  const instantiatedHeritage = InstantiateTypes(context, { callstack: [] }, heritage);
  const instantiatedProperties = InstantiateProperties({}, { callstack: [] }, properties4);
  const evaluatedInterface = EvaluateIntersect([...instantiatedHeritage, _Object_(instantiatedProperties)]);
  return evaluatedInterface;
}
function CyclicDefinitions(context, dependencies3) {
  const keys = exports_guard.Keys(context).filter((key) => dependencies3.includes(key));
  return keys.reduce((result, key) => {
    const type3 = context[key];
    const instantiatedType = IsInterfaceDeferred(type3) ? CyclicInterface(context, type3.parameters[0], type3.parameters[1]) : type3;
    return { ...result, [key]: instantiatedType };
  }, {});
}
function InstantiateCyclic(context, ref5, type3) {
  const dependencies3 = CyclicDependencies(context, ref5, type3);
  const definitions = CyclicDefinitions(context, dependencies3);
  const result = Cyclic(definitions, ref5);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/cyclic/target.mjs
function Resolve(defs2, ref5) {
  return ref5 in defs2 ? IsRef2(defs2[ref5]) ? Resolve(defs2, defs2[ref5].$ref) : defs2[ref5] : Never();
}
function CyclicTarget(defs2, ref5) {
  const result = Resolve(defs2, ref5);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/extends/extends.mjs
function Canonical(type3) {
  return IsCyclic(type3) ? CyclicExtends(type3) : IsUnsafe(type3) ? Unknown() : type3;
}
function Extends(inferred, left, right) {
  const canonicalLeft = Canonical(left);
  const canonicalRight = Canonical(right);
  return ExtendsLeft(inferred, canonicalLeft, canonicalRight);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/compare.mjs
var ResultEqual = "equal";
var ResultDisjoint = "disjoint";
var ResultLeftInside = "left-inside";
var ResultRightInside = "right-inside";
function Compare(left, right) {
  const extendsCheck = [
    IsUnknown(left) ? exports_result.ExtendsFalse() : Extends({}, left, right),
    IsUnknown(left) ? exports_result.ExtendsTrue({}) : Extends({}, right, left)
  ];
  return exports_result.IsExtendsTrueLike(extendsCheck[0]) && exports_result.IsExtendsTrueLike(extendsCheck[1]) ? ResultEqual : exports_result.IsExtendsTrueLike(extendsCheck[0]) && exports_result.IsExtendsFalse(extendsCheck[1]) ? ResultLeftInside : exports_result.IsExtendsFalse(extendsCheck[0]) && exports_result.IsExtendsTrueLike(extendsCheck[1]) ? ResultRightInside : ResultDisjoint;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/broaden.mjs
function BroadFilter(type3, types2) {
  return types2.filter((left) => {
    return Compare(type3, left) === ResultRightInside ? false : true;
  });
}
function IsBroadestType(type3, types2) {
  const result = types2.some((left) => {
    const result2 = Compare(type3, left);
    return exports_guard.IsEqual(result2, ResultLeftInside) || exports_guard.IsEqual(result2, ResultEqual);
  });
  return exports_guard.IsEqual(result, false);
}
function BroadenType(type3, types2) {
  const evaluated = EvaluateType(type3);
  return IsAny(evaluated) ? [evaluated] : IsBroadestType(evaluated, types2) ? [...BroadFilter(evaluated, types2), evaluated] : types2;
}
function BroadenTypes(types2) {
  return types2.reduce((result, left) => {
    return IsObject3(left) ? [...result, left] : IsNever(left) ? result : BroadenType(left, result);
  }, []);
}
function Broaden(types2) {
  const broadened = BroadenTypes(types2);
  const flattened = Flatten(broadened);
  const result = flattened.length === 0 ? Never() : flattened.length === 1 ? flattened[0] : Union(flattened);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/evaluate/instantiate.mjs
function EvaluateAction(type3, options) {
  const result = exports_memory.Update(EvaluateType(type3), {}, options);
  return result;
}
function EvaluateInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return EvaluateAction(instantiatedType, options);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/call/distribute_arguments.mjs
function CollectDistributionNames(expression, result = []) {
  return IsDeferred(expression) && exports_guard.IsEqual(expression.action, "Conditional") ? IsRef2(expression.parameters[0]) ? CollectDistributionNames(expression.parameters[2], CollectDistributionNames(expression.parameters[3], [...result, expression.parameters[0]["$ref"]])) : CollectDistributionNames(expression.parameters[2], CollectDistributionNames(expression.parameters[3], result)) : IsDeferred(expression) && exports_guard.IsEqual(expression.action, "Mapped") ? IsDeferred(expression.parameters[1]) && exports_guard.IsEqual(expression.parameters[1].action, "KeyOf") && IsRef2(expression.parameters[1].parameters[0]) ? [...result, expression.parameters[1].parameters[0]["$ref"]] : result : result;
}
function BuildDistributionArray(parameters, names) {
  return parameters.reduce((result, left) => [...result, names.includes(left.name)], []);
}
function ZipDistributionArray(arguments_, distributionArray, result = []) {
  return exports_guard.TakeLeft(arguments_, (argumentLeft, argumentRight) => exports_guard.TakeLeft(distributionArray, (booleanLeft, booleanRight) => ZipDistributionArray(argumentRight, booleanRight, [...result, [booleanLeft, argumentLeft]]), () => result), () => result);
}
function Expand(type3) {
  return IsUnion(type3) ? [...type3.anyOf] : [type3];
}
function Append(current, type3) {
  return current.reduce((result, left) => [...result, [...left, type3]], []);
}
function Cross(current, variants) {
  return variants.reduce((result, left) => {
    return [...result, ...Append(current, left)];
  }, []);
}
function Distribute2(zipped) {
  return zipped.reduce((result, left) => {
    return exports_guard.IsEqual(left[0], true) ? Cross(result, Expand(left[1])) : Cross(result, [left[1]]);
  }, [[]]);
}
function DistributeArguments(parameters, arguments_, expression) {
  const distributionNames = CollectDistributionNames(expression);
  const distributionArray = BuildDistributionArray(parameters, distributionNames);
  const zippedArguments = ZipDistributionArray(arguments_, distributionArray);
  return IsDeferred(expression) && exports_guard.IsEqual(expression.action, "Conditional") ? Distribute2(zippedArguments) : IsDeferred(expression) && exports_guard.IsEqual(expression.action, "Mapped") ? Distribute2(zippedArguments) : [arguments_];
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/call/resolve_target.mjs
function FromNotResolvable() {
  return ["(not-resolvable)", Never()];
}
function FromNotGeneric() {
  return ["(not-generic)", Never()];
}
function FromGeneric(name, parameters, expression) {
  return [name, Generic(parameters, expression)];
}
function FromRef4(context, ref5, arguments_) {
  return ref5 in context ? FromType6(context, ref5, context[ref5], arguments_) : FromNotResolvable();
}
function FromType6(context, name, target2, arguments_) {
  return IsGeneric(target2) ? FromGeneric(name, target2.parameters, target2.expression) : IsRef2(target2) ? FromRef4(context, target2.$ref, arguments_) : FromNotGeneric();
}
function ResolveTarget(context, target2, arguments_) {
  return FromType6(context, "(anonymous)", target2, arguments_);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/call/resolve_arguments.mjs
function AssertArgumentExtends(name, type3, extends_) {
  if (IsInfer(type3) || IsCall(type3) || exports_result.IsExtendsTrueLike(Extends({}, type3, extends_)))
    return;
  const cause = { parameter: name, expect: extends_, actual: type3 };
  throw new Error(`Argument for parameter ${name} does not satisfy constraint`, { cause });
}
function BindArgument(context, state2, name, extends_, type3) {
  const instantiatedArgument = InstantiateType(context, state2, type3);
  AssertArgumentExtends(name, instantiatedArgument, extends_);
  return exports_memory.Assign(context, { [name]: instantiatedArgument });
}
function BindArguments(context, state2, parameterLeft, parameterRight, arguments_) {
  const instantiatedExtends = InstantiateType(context, state2, parameterLeft.extends);
  const instantiatedEquals = InstantiateType(context, state2, parameterLeft.equals);
  return exports_guard.TakeLeft(arguments_, (left, right) => BindParameters(BindArgument(context, state2, parameterLeft["name"], instantiatedExtends, left), state2, parameterRight, right), () => BindParameters(BindArgument(context, state2, parameterLeft["name"], instantiatedExtends, instantiatedEquals), state2, parameterRight, []));
}
function BindParameters(context, state2, parameters, arguments_) {
  return exports_guard.TakeLeft(parameters, (left, right) => BindArguments(context, state2, left, right, arguments_), () => context);
}
function ResolveArgumentsContext(context, state2, parameters, arguments_) {
  return BindParameters(context, state2, parameters, arguments_);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/call/instantiate.mjs
function Peek(state2) {
  const result = exports_guard.IsGreaterThan(state2.callstack.length, 0) ? state2.callstack[state2.callstack.length - 1] : "";
  return result;
}
function IsTailCall(state2, name) {
  const result = exports_guard.IsEqual(Peek(state2), name);
  return result;
}
function CallDispatch(context, state2, target2, parameters, expression, arguments_) {
  const argumentsContext = ResolveArgumentsContext(context, state2, parameters, arguments_);
  const returnType = InstantiateType(argumentsContext, { callstack: [...state2.callstack, target2.$ref] }, expression);
  return InstantiateType(context, state2, returnType);
}
function CallDistributed(context, state2, target2, parameters, expression, distributedArguments) {
  return distributedArguments.reduce((result, arguments_) => [...result, CallDispatch(context, state2, target2, parameters, expression, arguments_)], []);
}
function CallImmediate(context, state2, target2, parameters, expression, arguments_) {
  const distributedArguments = DistributeArguments(parameters, arguments_, expression);
  const returnTypes = CallDistributed(context, state2, target2, parameters, expression, distributedArguments);
  const result = exports_guard.IsEqual(returnTypes.length, 1) ? returnTypes[0] : EvaluateUnion(returnTypes);
  return result;
}
function CallInstantiate(context, state2, target2, arguments_) {
  const instantiatedArguments = InstantiateTypes(context, state2, arguments_);
  const resolved = ResolveTarget(context, target2, arguments_);
  const name = resolved[0];
  const type3 = resolved[1];
  const result = IsGeneric(type3) ? IsTailCall(state2, name) ? CallConstruct(Ref2(name), instantiatedArguments) : CallImmediate(context, state2, Ref2(name), type3.parameters, type3.expression, instantiatedArguments) : CallConstruct(target2, instantiatedArguments);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/types/call.mjs
function CallConstruct(target2, arguments_) {
  return exports_memory.Create({ ["~kind"]: "Call" }, { target: target2, arguments: arguments_ }, {});
}
function IsCall(value) {
  return IsKind(value, "Call");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/mapping.mjs
function ApplyMapping(mapping, value) {
  return mapping(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/from_literal.mjs
function FromLiteral3(mapping, value) {
  return exports_guard.IsString(value) ? Literal(ApplyMapping(mapping, value)) : Literal(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/from_template_literal.mjs
function FromTemplateLiteral(mapping, pattern3) {
  const decoded = TemplateLiteralDecode(pattern3);
  const result = FromType7(mapping, decoded);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/from_union.mjs
function FromUnion2(mapping, types2) {
  const result = types2.map((type3) => FromType7(mapping, type3));
  return Union(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/from_type.mjs
function FromType7(mapping, type3) {
  return IsLiteral(type3) ? FromLiteral3(mapping, type3.const) : IsTemplateLiteral(type3) ? FromTemplateLiteral(mapping, type3.pattern) : IsUnion(type3) ? FromUnion2(mapping, type3.anyOf) : type3;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/capitalize.mjs
function CapitalizeDeferred(type3, options = {}) {
  return Deferred("Capitalize", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/lowercase.mjs
function LowercaseDeferred(type3, options = {}) {
  return Deferred("Lowercase", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/uncapitalize.mjs
function UncapitalizeDeferred(type3, options = {}) {
  return Deferred("Uncapitalize", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/uppercase.mjs
function UppercaseDeferred(type3, options = {}) {
  return Deferred("Uppercase", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/intrinsics/instantiate.mjs
var CapitalizeMapping = (input) => input[0].toUpperCase() + input.slice(1);
var LowercaseMapping = (input) => input.toLowerCase();
var UncapitalizeMapping = (input) => input[0].toLowerCase() + input.slice(1);
var UppercaseMapping = (input) => input.toUpperCase();
function CapitalizeAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType7(CapitalizeMapping, type3), {}, options) : CapitalizeDeferred(type3, options);
  return result;
}
function LowercaseAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType7(LowercaseMapping, type3), {}, options) : LowercaseDeferred(type3, options);
  return result;
}
function UncapitalizeAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType7(UncapitalizeMapping, type3), {}, options) : UncapitalizeDeferred(type3, options);
  return result;
}
function UppercaseAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType7(UppercaseMapping, type3), {}, options) : UppercaseDeferred(type3, options);
  return result;
}
function CapitalizeInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return CapitalizeAction(instantiatedType, options);
}
function LowercaseInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return LowercaseAction(instantiatedType, options);
}
function UncapitalizeInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return UncapitalizeAction(instantiatedType, options);
}
function UppercaseInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return UppercaseAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/conditional.mjs
function ConditionalDeferred(left, right, true_, false_, options = {}) {
  return Deferred("Conditional", [left, right, true_, false_], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/conditional/instantiate.mjs
function ConditionalOperation(context, state2, left, right, true_, false_) {
  const extendsResult = Extends(context, left, right);
  return exports_result.IsExtendsUnion(extendsResult) ? Union([InstantiateType(extendsResult.inferred, state2, true_), InstantiateType(context, state2, false_)]) : exports_result.IsExtendsTrue(extendsResult) ? InstantiateType(extendsResult.inferred, state2, true_) : InstantiateType(context, state2, false_);
}
function ConditionalAction(context, state2, left, right, true_, false_, options) {
  const result = CanInstantiate([left, right]) ? exports_memory.Update(ConditionalOperation(context, state2, left, right, true_, false_), {}, options) : ConditionalDeferred(left, right, true_, false_, options);
  return result;
}
function ConditionalInstantiate(context, state2, left, right, true_, false_, options) {
  const instantiatedLeft = InstantiateType(context, state2, left);
  const instantiatedRight = InstantiateType(context, state2, right);
  return ConditionalAction(context, state2, instantiatedLeft, instantiatedRight, true_, false_, options);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/constructor_parameters.mjs
function ConstructorParametersDeferred(type3, options = {}) {
  return Deferred("ConstructorParameters", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/constructor_parameters/instantiate.mjs
function ConstructorParametersOperation(type3) {
  const parameters = IsConstructor3(type3) ? type3["parameters"] : [];
  const instantiatedParameters = InstantiateElements({}, { callstack: [] }, parameters);
  const result = Tuple(instantiatedParameters);
  return result;
}
function ConstructorParametersAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(ConstructorParametersOperation(type3), {}, options) : ConstructorParametersDeferred(type3, options);
  return result;
}
function ConstructorParametersInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return ConstructorParametersAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/exclude.mjs
function ExcludeDeferred(left, right, options = {}) {
  return Deferred("Exclude", [left, right], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/exclude/operation.mjs
function ExcludeUnionLeft(types2, right) {
  return types2.reduce((result, head) => {
    return [...result, ...ExcludeTypeLeft(head, right)];
  }, []);
}
function ExcludeTypeLeft(left, right) {
  const check3 = Extends({}, left, right);
  const result = exports_result.IsExtendsTrueLike(check3) ? [] : [left];
  return result;
}
function ExcludeOperation(left, right) {
  const remaining = IsEnum2(left) ? ExcludeUnionLeft(EnumValuesToVariants(left.enum), right) : IsUnion(left) ? ExcludeUnionLeft(Flatten(left.anyOf), right) : ExcludeTypeLeft(left, right);
  const result = EvaluateUnion(remaining);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/exclude/instantiate.mjs
function ExcludeAction(left, right, options) {
  const result = CanInstantiate([left, right]) ? exports_memory.Update(ExcludeOperation(left, right), {}, options) : ExcludeDeferred(left, right, options);
  return result;
}
function ExcludeInstantiate(context, state2, left, right, options) {
  const instantiatedLeft = InstantiateType(context, state2, left);
  const instantiatedRight = InstantiateType(context, state2, right);
  return ExcludeAction(instantiatedLeft, instantiatedRight, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/extract.mjs
function ExtractDeferred(left, right, options = {}) {
  return Deferred("Extract", [left, right], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/extract/operation.mjs
function ExtractUnionLeft(types2, right) {
  return types2.reduce((result, head) => {
    return [...result, ...ExtractTypeLeft(head, right)];
  }, []);
}
function ExtractTypeLeft(left, right) {
  const check3 = Extends({}, left, right);
  const result = exports_result.IsExtendsTrueLike(check3) ? [left] : [];
  return result;
}
function ExtractOperation(left, right) {
  const remaining = IsEnum2(left) ? ExtractUnionLeft(EnumValuesToVariants(left.enum), right) : IsUnion(left) ? ExtractUnionLeft(Flatten(left.anyOf), right) : ExtractTypeLeft(left, right);
  const result = EvaluateUnion(remaining);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/extract/instantiate.mjs
function ExtractAction(left, right, options) {
  const result = CanInstantiate([left, right]) ? exports_memory.Update(ExtractOperation(left, right), {}, options) : ExtractDeferred(left, right, options);
  return result;
}
function ExtractInstantiate(context, state2, left, right, options) {
  const instantiatedLeft = InstantiateType(context, state2, left);
  const instantiatedRight = InstantiateType(context, state2, right);
  return ExtractAction(instantiatedLeft, instantiatedRight, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/indexed.mjs
function IndexDeferred(type3, indexer, options = {}) {
  return Deferred("Index", [type3, indexer], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_cyclic.mjs
function FromCyclic(defs2, ref5) {
  const target2 = CyclicTarget(defs2, ref5);
  const result = FromType8(target2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_intersect.mjs
function CollapseIntersectProperties(left, right) {
  const leftKeys = exports_guard.Keys(left).filter((key) => !exports_guard.HasPropertyKey(right, key));
  const rightKeys = exports_guard.Keys(right).filter((key) => !exports_guard.HasPropertyKey(left, key));
  const sharedKeys = exports_guard.Keys(left).filter((key) => exports_guard.HasPropertyKey(right, key));
  const leftProperties = leftKeys.reduce((result, key) => ({ ...result, [key]: left[key] }), {});
  const rightProperties = rightKeys.reduce((result, key) => ({ ...result, [key]: right[key] }), {});
  const sharedProperties = sharedKeys.reduce((result, key) => ({ ...result, [key]: EvaluateIntersect([left[key], right[key]]) }), {});
  const unique = exports_memory.Assign(leftProperties, rightProperties);
  const shared = exports_memory.Assign(unique, sharedProperties);
  return shared;
}
function FromIntersect(types2) {
  return types2.reduce((result, left) => {
    return CollapseIntersectProperties(result, FromType8(left));
  }, {});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_object.mjs
function FromObject4(properties4) {
  return properties4;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_tuple.mjs
function FromTuple(types2) {
  const object2 = TupleToObject(Tuple(types2));
  const result = FromType8(object2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_union.mjs
function CollapseUnionProperties(left, right) {
  const sharedKeys = exports_guard.Keys(left).filter((key) => (key in right));
  const result = sharedKeys.reduce((result2, key) => {
    return { ...result2, [key]: EvaluateUnion([left[key], right[key]]) };
  }, {});
  return result;
}
function ReduceVariants(types2, result) {
  return exports_guard.TakeLeft(types2, (left, right) => ReduceVariants(right, CollapseUnionProperties(result, FromType8(left))), () => result);
}
function FromUnion3(types2) {
  return exports_guard.TakeLeft(types2, (left, right) => ReduceVariants(right, FromType8(left)), () => Unreachable());
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/from_type.mjs
function FromType8(type3) {
  return IsCyclic(type3) ? FromCyclic(type3.$defs, type3.$ref) : IsIntersect(type3) ? FromIntersect(type3.allOf) : IsUnion(type3) ? FromUnion3(type3.anyOf) : IsTuple(type3) ? FromTuple(type3.items) : IsObject3(type3) ? FromObject4(type3.properties) : {};
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/object/collapse.mjs
function CollapseToObject(type3) {
  const properties4 = FromType8(type3);
  const result = _Object_(properties4);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/helpers/keys.mjs
var integerKeyPattern = new RegExp("^(?:0|[1-9][0-9]*)$");
function ConvertToIntegerKey(value) {
  const normal = `${value}`;
  return integerKeyPattern.test(normal) ? parseInt(normal) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/from_array.mjs
function NormalizeLiteral(value) {
  return Literal(ConvertToIntegerKey(value));
}
function NormalizeIndexerTypes(types2) {
  return types2.map((type3) => NormalizeIndexer(type3));
}
function NormalizeIndexer(type3) {
  return IsIntersect(type3) ? Intersect(NormalizeIndexerTypes(type3.allOf)) : IsUnion(type3) ? Union(NormalizeIndexerTypes(type3.anyOf)) : IsLiteral(type3) ? NormalizeLiteral(type3.const) : type3;
}
function FromArray4(type3, indexer) {
  const normalizedIndexer = NormalizeIndexer(indexer);
  const check3 = Extends({}, normalizedIndexer, Number2());
  const result = exports_result.IsExtendsTrueLike(check3) ? type3 : IsLiteral(indexer) && exports_guard.IsEqual(indexer.const, "length") ? Number2() : Never();
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_cyclic.mjs
function FromCyclic2(defs2, ref5) {
  const target2 = CyclicTarget(defs2, ref5);
  const result = FromType9(target2);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_union.mjs
function FromUnion4(types2) {
  return types2.reduce((result, left) => {
    return [...result, ...FromType9(left)];
  }, []);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_enum.mjs
function FromEnum(values) {
  const variants = EnumValuesToVariants(values);
  const result = FromUnion4(variants);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_intersect.mjs
function FromIntersect2(types2) {
  const evaluated = EvaluateIntersect(types2);
  const result = FromType9(evaluated);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_literal.mjs
function FromLiteral4(value) {
  const result = [`${value}`];
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_template_literal.mjs
function FromTemplateLiteral2(pattern3) {
  const decoded = TemplateLiteralDecode(pattern3);
  const result = FromType9(decoded);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/from_type.mjs
function FromType9(type3) {
  return IsCyclic(type3) ? FromCyclic2(type3.$defs, type3.$ref) : IsEnum2(type3) ? FromEnum(type3.enum) : IsIntersect(type3) ? FromIntersect2(type3.allOf) : IsLiteral(type3) ? FromLiteral4(type3.const) : IsTemplateLiteral(type3) ? FromTemplateLiteral2(type3.pattern) : IsUnion(type3) ? FromUnion4(type3.anyOf) : [];
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/to_indexable_keys.mjs
function ToIndexableKeys(type3) {
  const result = FromType9(type3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/this/expand_this.mjs
function FromTypes5(properties4, types2) {
  return types2.map((type3) => FromType10(properties4, type3));
}
function FromType10(properties4, type3) {
  return IsArray3(type3) ? _Array_(FromType10(properties4, type3.items)) : IsAsyncIterator3(type3) ? AsyncIterator(FromType10(properties4, type3.iteratorItems)) : IsConstructor3(type3) ? Constructor(FromTypes5(properties4, type3.parameters), FromType10(properties4, type3.instanceType)) : IsFunction3(type3) ? _Function_(FromTypes5(properties4, type3.parameters), FromType10(properties4, type3.returnType)) : IsIterator3(type3) ? Iterator(FromType10(properties4, type3.iteratorItems)) : IsPromise(type3) ? _Promise_(FromType10(properties4, type3.item)) : IsTuple(type3) ? Tuple(FromTypes5(properties4, type3.items)) : IsUnion(type3) ? Union(FromTypes5(properties4, type3.anyOf)) : IsIntersect(type3) ? Intersect(FromTypes5(properties4, type3.allOf)) : IsThis(type3) ? _Object_(properties4) : type3;
}
function ExpandThis(properties4, type3) {
  const result = FromType10(properties4, type3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/from_object.mjs
function IndexProperty(properties4, key) {
  const selectedType = key in properties4 ? properties4[key] : Never();
  const result = ExpandThis(properties4, selectedType);
  return result;
}
function IndexProperties(properties4, keys) {
  return keys.reduce((result, left) => {
    return [...result, IndexProperty(properties4, left)];
  }, []);
}
function FromIndexer(properties4, indexer) {
  const keys = ToIndexableKeys(indexer);
  const variants = IndexProperties(properties4, keys);
  const result = EvaluateUnion(variants);
  return result;
}
var NumericKeyPattern = new RegExp(IntegerKey);
function NumericKeys(keys) {
  const result = keys.filter((key) => NumericKeyPattern.test(key));
  return result;
}
function FromIndexerNumber(properties4) {
  const keys = PropertyKeys(properties4);
  const numericKeys = NumericKeys(keys);
  const variants = IndexProperties(properties4, numericKeys);
  const result = EvaluateUnion(variants);
  return result;
}
function FromObject5(properties4, indexer) {
  const result = IsNumber4(indexer) ? FromIndexerNumber(properties4) : FromIndexer(properties4, indexer);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/array_indexer.mjs
function ConvertLiteral(value) {
  return Literal(ConvertToIntegerKey(value));
}
function ArrayIndexerTypes(types2) {
  return types2.map((type3) => FormatArrayIndexer(type3));
}
function FormatArrayIndexer(type3) {
  return IsIntersect(type3) ? Intersect(ArrayIndexerTypes(type3.allOf)) : IsUnion(type3) ? Union(ArrayIndexerTypes(type3.anyOf)) : IsLiteral(type3) ? ConvertLiteral(type3.const) : type3;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/from_tuple.mjs
function IndexElementsWithIndexer(types2, indexer) {
  return types2.reduceRight((result, right, index2) => {
    const check3 = Extends({}, Literal(index2), indexer);
    return exports_result.IsExtendsTrueLike(check3) ? [right, ...result] : result;
  }, []);
}
function FromTupleWithIndexer(types2, indexer) {
  const formattedArrayIndexer = FormatArrayIndexer(indexer);
  const elements = IndexElementsWithIndexer(types2, formattedArrayIndexer);
  return EvaluateUnionFast(elements);
}
function FromTupleWithoutIndexer(types2) {
  return EvaluateUnionFast(types2);
}
function FromTuple2(types2, indexer) {
  return IsLiteral(indexer) && exports_guard.IsEqual(indexer.const, "length") ? Literal(types2.length) : IsNumber4(indexer) || IsInteger3(indexer) ? FromTupleWithoutIndexer(types2) : FromTupleWithIndexer(types2, indexer);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/from_type.mjs
function FromType11(type3, indexer) {
  return IsArray3(type3) ? FromArray4(type3.items, indexer) : IsObject3(type3) ? FromObject5(type3.properties, indexer) : IsTuple(type3) ? FromTuple2(type3.items, indexer) : Never();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexed/instantiate.mjs
function NormalizeType(type3) {
  const result = IsCyclic(type3) || IsIntersect(type3) || IsUnion(type3) ? CollapseToObject(type3) : type3;
  return result;
}
function IndexAction(type3, indexer, options) {
  const result = CanInstantiate([type3, indexer]) ? exports_memory.Update(FromType11(NormalizeType(type3), indexer), {}, options) : IndexDeferred(type3, indexer, options);
  return result;
}
function IndexInstantiate(context, state2, type3, indexer, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  const instantiatedIndexer = InstantiateType(context, state2, indexer);
  return IndexAction(instantiatedType, instantiatedIndexer, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/instance_type.mjs
function InstanceTypeDeferred(type3, options = {}) {
  return Deferred("InstanceType", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/instance_type/instantiate.mjs
function InstanceTypeOperation(type3) {
  return IsConstructor3(type3) ? type3["instanceType"] : Never();
}
function InstanceTypeAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(InstanceTypeOperation(type3), {}, options) : InstanceTypeDeferred(type3, options);
  return result;
}
function InstanceTypeInstantiate(context, state2, type3, options = {}) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return InstanceTypeAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/keyof.mjs
function KeyOfDeferred(type3, options = {}) {
  return Deferred("KeyOf", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_any.mjs
function FromAny() {
  return Union([Number2(), String2(), Symbol2()]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_array.mjs
function FromArray5(_type) {
  return Number2();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_object.mjs
function FromPropertyKeys(keys) {
  const result = keys.reduce((result2, left) => {
    return IsLiteralValue(left) ? [...result2, Literal(ConvertToIntegerKey(left))] : Unreachable();
  }, []);
  return result;
}
function FromObject6(properties4) {
  const propertyKeys = exports_guard.Keys(properties4);
  const variants = FromPropertyKeys(propertyKeys);
  const result = EvaluateUnionFast(variants);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_record.mjs
function FromRecord(type3) {
  return RecordKey(type3);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_tuple.mjs
function FromTuple3(types2) {
  const result = types2.map((_, index2) => Literal(index2));
  return EvaluateUnionFast(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/from_type.mjs
function FromType12(type3) {
  return IsAny(type3) ? FromAny() : IsArray3(type3) ? FromArray5(type3.items) : IsObject3(type3) ? FromObject6(type3.properties) : IsRecord(type3) ? FromRecord(type3) : IsTuple(type3) ? FromTuple3(type3.items) : Never();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/keyof/instantiate.mjs
function NormalizeType2(type3) {
  const result = IsCyclic(type3) || IsIntersect(type3) || IsUnion(type3) ? CollapseToObject(type3) : type3;
  return result;
}
function KeyOfAction(type3, options) {
  return CanInstantiate([type3]) ? exports_memory.Update(FromType12(NormalizeType2(type3)), {}, options) : KeyOfDeferred(type3, options);
}
function KeyOfInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return KeyOfAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/mapped.mjs
function MappedDeferred(identifier2, type3, as, property, options = {}) {
  return Deferred("Mapped", [identifier2, type3, as, property], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/mapped/mapped_variants.mjs
function FromTemplateLiteral3(pattern3) {
  const decoded = TemplateLiteralDecode(pattern3);
  const result = FromType13(decoded);
  return result;
}
function FromUnion5(types2) {
  return types2.reduce((result, left) => {
    return [...result, ...FromType13(left)];
  }, []);
}
function FromLiteral5(value) {
  const result = exports_guard.IsNumber(value) ? [Literal(`${value}`)] : [Literal(value)];
  return result;
}
function FromType13(type3) {
  const result = IsEnum2(type3) ? FromUnion5(EnumValuesToVariants(type3.enum)) : IsLiteral(type3) ? FromLiteral5(type3.const) : IsTemplateLiteral(type3) ? FromTemplateLiteral3(type3.pattern) : IsUnion(type3) ? FromUnion5(type3.anyOf) : [type3];
  return result;
}
function MappedVariants(type3) {
  const result = FromType13(type3);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/mapped/mapped_operation.mjs
function CanonicalAs(instantiatedAs) {
  const result = IsTemplateLiteral(instantiatedAs) ? TemplateLiteralDecode(instantiatedAs.pattern) : instantiatedAs;
  return result;
}
function MappedVariant(context, state2, identifier2, variant, as, property) {
  const variantContext = exports_memory.Assign(context, { [identifier2["name"]]: variant });
  const instantiatedAs = InstantiateType(variantContext, state2, as);
  const canonicalAs = CanonicalAs(instantiatedAs);
  const instantiatedProperty = InstantiateType(variantContext, state2, property);
  return IsLiteralNumber(canonicalAs) || IsLiteralString(canonicalAs) ? { [canonicalAs.const]: instantiatedProperty } : {};
}
function MappedProperties(context, state2, identifier2, variants, as, property) {
  return variants.reduce((result, left) => {
    return [...result, MappedVariant(context, state2, identifier2, left, as, property)];
  }, []);
}
function MappedObjects(properties4) {
  return properties4.reduce((result, left) => {
    return [...result, _Object_(left)];
  }, []);
}
function MappedOperation(context, state2, identifier2, type3, as, property) {
  const variants = MappedVariants(type3);
  const mappedProperties = MappedProperties(context, state2, identifier2, variants, as, property);
  const mappedObjects = MappedObjects(mappedProperties);
  const result = EvaluateIntersect(mappedObjects);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/mapped/instantiate.mjs
function MappedAction(context, state2, identifier2, type3, as, property, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(MappedOperation(context, state2, identifier2, type3, as, property), {}, options) : MappedDeferred(identifier2, type3, as, property, options);
  return result;
}
function MappedInstantiate(context, state2, identifier2, type3, as, property, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return MappedAction(context, state2, identifier2, instantiatedType, as, property, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/module/instantiate.mjs
function InstantiateCyclics(context, cyclicKeys) {
  const keys = exports_guard.Keys(context).filter((key) => cyclicKeys.includes(key));
  return keys.reduce((result, key) => {
    return { ...result, [key]: InstantiateCyclic(context, key, context[key]) };
  }, {});
}
function InstantiateNonCyclics(context, cyclicKeys) {
  const keys = exports_guard.Keys(context).filter((key) => !cyclicKeys.includes(key));
  return keys.reduce((result, key) => {
    return { ...result, [key]: InstantiateType(context, { callstack: [] }, context[key]) };
  }, {});
}
function InstantiateModule(context, options) {
  const cyclicCandidates = CyclicCandidates(context);
  const instantiatedCyclics = InstantiateCyclics(context, cyclicCandidates);
  const instantiatedNonCyclics = InstantiateNonCyclics(context, cyclicCandidates);
  const instantiatedModule = { ...instantiatedCyclics, ...instantiatedNonCyclics };
  return exports_memory.Update(instantiatedModule, {}, options);
}
function ModuleInstantiate(context, _state, properties4, options) {
  const moduleContext = exports_memory.Assign(context, properties4);
  const instantiatedModule = InstantiateModule(moduleContext, options);
  return instantiatedModule;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/non_nullable.mjs
function NonNullableDeferred(type3, options = {}) {
  return Deferred("NonNullable", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/non_nullable/instantiate.mjs
function NonNullableOperation(type3) {
  const excluded = Union([Null(), Undefined()]);
  return ExcludeAction(type3, excluded, {});
}
function NonNullableAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(NonNullableOperation(type3), {}, options) : NonNullableDeferred(type3, options);
  return result;
}
function NonNullableInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return NonNullableAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/omit.mjs
function OmitDeferred(type3, indexer, options = {}) {
  return Deferred("Omit", [type3, indexer], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/indexable/to_indexable.mjs
function ToIndexable(type3) {
  const collapsed = CollapseToObject(type3);
  const result = IsObject3(collapsed) ? collapsed.properties : Unreachable();
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/omit/from_type.mjs
function FromKeys(properties4, keys) {
  const result = exports_guard.Keys(properties4).reduce((result2, key) => {
    return keys.includes(key) ? result2 : { ...result2, [key]: properties4[key] };
  }, {});
  return result;
}
function FromType14(type3, indexer) {
  const indexable = ToIndexable(type3);
  const indexableKeys = ToIndexableKeys(indexer);
  const omitted = FromKeys(indexable, indexableKeys);
  const result = _Object_(omitted);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/omit/instantiate.mjs
function OmitAction(type3, indexer, options) {
  const result = CanInstantiate([type3, indexer]) ? exports_memory.Update(FromType14(type3, indexer), {}, options) : OmitDeferred(type3, indexer, options);
  return result;
}
function OmitInstantiate(context, state2, type3, indexer, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  const instantiatedIndexer = InstantiateType(context, state2, indexer);
  return OmitAction(instantiatedType, instantiatedIndexer, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/options.mjs
function OptionsDeferred(type3, options) {
  return Deferred("Options", [type3, options], {});
}
function Options(type3, options) {
  return OptionsAction(type3, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/options/instantiate.mjs
function OptionsAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(type3, {}, options) : OptionsDeferred(type3, options);
  return result;
}
function OptionsInstantiate(context, state2, type3, options) {
  const instaniatedType = InstantiateType(context, state2, type3);
  return OptionsAction(instaniatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/parameters.mjs
function ParametersDeferred(type3, options = {}) {
  return Deferred("Parameters", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/parameters/instantiate.mjs
function ParametersOperation(type3) {
  const parameters = IsFunction3(type3) ? type3["parameters"] : [];
  const instantiatedParameters = InstantiateElements({}, { callstack: [] }, parameters);
  const result = Tuple(instantiatedParameters);
  return result;
}
function ParametersAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(ParametersOperation(type3), {}, options) : ParametersDeferred(type3, options);
  return result;
}
function ParametersInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return ParametersAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/partial.mjs
function PartialDeferred(type3, options = {}) {
  return Deferred("Partial", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/from_cyclic.mjs
function FromCyclic3(defs2, ref5) {
  const target2 = CyclicTarget(defs2, ref5);
  const partial = FromType15(target2);
  const result = Cyclic(exports_memory.Assign(defs2, { [ref5]: partial }), ref5);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/from_intersect.mjs
function FromIntersect3(types2) {
  const result = types2.map((type3) => FromType15(type3));
  return EvaluateIntersect(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/from_union.mjs
function FromUnion6(types2) {
  const result = types2.map((type3) => FromType15(type3));
  return Union(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/from_object.mjs
function FromObject7(properties4) {
  const mapped = exports_guard.Keys(properties4).reduce((result2, left) => {
    return { ...result2, [left]: Optional(properties4[left]) };
  }, {});
  const result = _Object_(mapped);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/from_type.mjs
function FromType15(type3) {
  return IsCyclic(type3) ? FromCyclic3(type3.$defs, type3.$ref) : IsIntersect(type3) ? FromIntersect3(type3.allOf) : IsUnion(type3) ? FromUnion6(type3.anyOf) : IsObject3(type3) ? FromObject7(type3.properties) : _Object_({});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/partial/instantiate.mjs
function PartialAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType15(type3), {}, options) : PartialDeferred(type3, options);
  return result;
}
function PartialInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return PartialAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/pick.mjs
function PickDeferred(type3, indexer, options = {}) {
  return Deferred("Pick", [type3, indexer], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/pick/from_type.mjs
function FromKeys2(properties4, keys) {
  const result = exports_guard.Keys(properties4).reduce((result2, key) => {
    return keys.includes(key) ? exports_memory.Assign(result2, { [key]: properties4[key] }) : result2;
  }, {});
  return result;
}
function FromType16(type3, indexer) {
  const indexable = ToIndexable(type3);
  const keys = ToIndexableKeys(indexer);
  const applied = FromKeys2(indexable, keys);
  const result = _Object_(applied);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/pick/instantiate.mjs
function PickAction(type3, indexer, options) {
  const result = CanInstantiate([type3, indexer]) ? exports_memory.Update(FromType16(type3, indexer), {}, options) : PickDeferred(type3, indexer, options);
  return result;
}
function PickInstantiate(context, state2, type3, indexer, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  const instantiatedIndexer = InstantiateType(context, state2, indexer);
  return PickAction(instantiatedType, instantiatedIndexer, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/readonly_object.mjs
function ReadonlyObjectDeferred(type3, options = {}) {
  return Deferred("ReadonlyObject", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_array.mjs
function FromArray6(type3) {
  const result = Immutable(_Array_(type3));
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_cyclic.mjs
function FromCyclic4(defs2, ref5) {
  const target2 = CyclicTarget(defs2, ref5);
  const partial = FromType17(target2);
  const result = Cyclic(exports_memory.Assign(defs2, { [ref5]: partial }), ref5);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_intersect.mjs
function FromIntersect4(types2) {
  const result = types2.map((type3) => FromType17(type3));
  return EvaluateIntersect(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_object.mjs
function FromObject8(properties4) {
  const mapped = exports_guard.Keys(properties4).reduce((result2, left) => {
    return { ...result2, [left]: Readonly(properties4[left]) };
  }, {});
  const result = _Object_(mapped);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_tuple.mjs
function FromTuple4(types2) {
  const result = Immutable(Tuple(types2));
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_union.mjs
function FromUnion7(types2) {
  const result = types2.map((type3) => FromType17(type3));
  return Union(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/from_type.mjs
function FromType17(type3) {
  return IsArray3(type3) ? FromArray6(type3.items) : IsCyclic(type3) ? FromCyclic4(type3.$defs, type3.$ref) : IsIntersect(type3) ? FromIntersect4(type3.allOf) : IsObject3(type3) ? FromObject8(type3.properties) : IsTuple(type3) ? FromTuple4(type3.items) : IsUnion(type3) ? FromUnion7(type3.anyOf) : type3;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/readonly_object/instantiate.mjs
function ReadonlyObjectAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType17(type3), {}, options) : ReadonlyObjectDeferred(type3);
  return result;
}
function ReadonlyObjectInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return ReadonlyObjectAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/ref/instantiate.mjs
function RefInstantiate(context, state2, type3, ref5) {
  return ref5 in context ? CyclicCheck([ref5], context, context[ref5]) ? type3 : InstantiateType(context, state2, context[ref5]) : type3;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/from_cyclic.mjs
function FromCyclic5(defs2, ref5) {
  const target2 = CyclicTarget(defs2, ref5);
  const partial = FromType18(target2);
  const result = Cyclic(exports_memory.Assign(defs2, { [ref5]: partial }), ref5);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/from_intersect.mjs
function FromIntersect5(types2) {
  const result = types2.map((type3) => FromType18(type3));
  return EvaluateIntersect(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/from_union.mjs
function FromUnion8(types2) {
  const result = types2.map((type3) => FromType18(type3));
  return Union(result);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/from_object.mjs
function FromObject9(properties4) {
  const mapped = exports_guard.Keys(properties4).reduce((result2, left) => {
    return { ...result2, [left]: OptionalRemove(properties4[left]) };
  }, {});
  const result = _Object_(mapped);
  return result;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/from_type.mjs
function FromType18(type3) {
  return IsCyclic(type3) ? FromCyclic5(type3.$defs, type3.$ref) : IsIntersect(type3) ? FromIntersect5(type3.allOf) : IsUnion(type3) ? FromUnion8(type3.anyOf) : IsObject3(type3) ? FromObject9(type3.properties) : _Object_({});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/required.mjs
function RequiredDeferred(type3, options = {}) {
  return Deferred("Required", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/required/instantiate.mjs
function RequiredAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(FromType18(type3), {}, options) : RequiredDeferred(type3, options);
  return result;
}
function RequiredInstantiate(context, state2, type3, options) {
  const instaniatedType = InstantiateType(context, state2, type3);
  return RequiredAction(instaniatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/return_type.mjs
function ReturnTypeDeferred(type3, options = {}) {
  return Deferred("ReturnType", [type3], options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/return_type/instantiate.mjs
function ReturnTypeOperation(type3) {
  return IsFunction3(type3) ? type3["returnType"] : Never();
}
function ReturnTypeAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(ReturnTypeOperation(type3), {}, options) : ReturnTypeDeferred(type3, options);
  return result;
}
function ReturnTypeInstantiate(context, state2, type3, options = {}) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return ReturnTypeAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/rest/spread.mjs
function SpreadElement(type3) {
  const result = IsRest(type3) ? IsTuple(type3.items) ? RestSpread(type3.items.items) : IsInfer(type3.items) ? [type3] : IsRef2(type3.items) ? [type3] : [Never()] : [type3];
  return result;
}
function RestSpread(types2) {
  const result = types2.reduce((result2, left) => {
    return [...result2, ...SpreadElement(left)];
  }, []);
  return result;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/instantiate.mjs
function CanInstantiate(types2) {
  return exports_guard.TakeLeft(types2, (left, right) => IsRef2(left) ? false : CanInstantiate(right), () => true);
}
function ModifierActions(type3, readonly, optional) {
  return IsReadonlyRemoveAction(type3) ? ModifierActions(type3.type, "remove", optional) : IsOptionalRemoveAction(type3) ? ModifierActions(type3.type, readonly, "remove") : IsReadonlyAddAction(type3) ? ModifierActions(type3.type, "add", optional) : IsOptionalAddAction(type3) ? ModifierActions(type3.type, readonly, "add") : [type3, readonly, optional];
}
function ApplyReadonly(action, type3) {
  return exports_guard.IsEqual(action, "remove") ? ReadonlyRemove(type3) : exports_guard.IsEqual(action, "add") ? ReadonlyAdd(type3) : type3;
}
function ApplyOptional(action, type3) {
  return exports_guard.IsEqual(action, "remove") ? OptionalRemove(type3) : exports_guard.IsEqual(action, "add") ? OptionalAdd(type3) : type3;
}
function InstantiateProperties(context, state2, properties4) {
  return exports_guard.Keys(properties4).reduce((result, key) => {
    return { ...result, [key]: InstantiateType(context, state2, properties4[key]) };
  }, {});
}
function InstantiateElements(context, state2, types2) {
  const elements = InstantiateTypes(context, state2, types2);
  const result = RestSpread(elements);
  return result;
}
function InstantiateTypes(context, state2, types2) {
  return types2.map((type3) => InstantiateType(context, state2, type3));
}
function InstantiateDeferred(context, state2, action, parameters, options) {
  return exports_guard.IsEqual(action, "Awaited") ? AwaitedInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Capitalize") ? CapitalizeInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Conditional") ? ConditionalInstantiate(context, state2, parameters[0], parameters[1], parameters[2], parameters[3], options) : exports_guard.IsEqual(action, "ConstructorParameters") ? ConstructorParametersInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Evaluate") ? EvaluateInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Exclude") ? ExcludeInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "Extract") ? ExtractInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "Index") ? IndexInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "InstanceType") ? InstanceTypeInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Interface") ? InterfaceInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "KeyOf") ? KeyOfInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Lowercase") ? LowercaseInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Mapped") ? MappedInstantiate(context, state2, parameters[0], parameters[1], parameters[2], parameters[3], options) : exports_guard.IsEqual(action, "Module") ? ModuleInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "NonNullable") ? NonNullableInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Pick") ? PickInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "Options") ? OptionsInstantiate(context, state2, parameters[0], parameters[1]) : exports_guard.IsEqual(action, "Parameters") ? ParametersInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Partial") ? PartialInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Omit") ? OmitInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "ReadonlyObject") ? ReadonlyObjectInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Record") ? RecordInstantiate(context, state2, parameters[0], parameters[1], options) : exports_guard.IsEqual(action, "Required") ? RequiredInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "ReturnType") ? ReturnTypeInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "TemplateLiteral") ? TemplateLiteralInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Uncapitalize") ? UncapitalizeInstantiate(context, state2, parameters[0], options) : exports_guard.IsEqual(action, "Uppercase") ? UppercaseInstantiate(context, state2, parameters[0], options) : Deferred(action, parameters, options);
}
function InstantiateType(context, state2, input) {
  const immutable = IsImmutable(input);
  const modifiers = ModifierActions(input, IsReadonly(input) ? "add" : "none", IsOptional(input) ? "add" : "none");
  const type3 = IsBase(modifiers[0]) ? modifiers[0].Clone() : modifiers[0];
  const instantiated = IsRef2(type3) ? RefInstantiate(context, state2, type3, type3.$ref) : IsArray3(type3) ? _Array_(InstantiateType(context, state2, type3.items), ArrayOptions(type3)) : IsAsyncIterator3(type3) ? AsyncIterator(InstantiateType(context, state2, type3.iteratorItems), AsyncIteratorOptions(type3)) : IsCall(type3) ? CallInstantiate(context, state2, type3.target, type3.arguments) : IsConstructor3(type3) ? Constructor(InstantiateTypes(context, state2, type3.parameters), InstantiateType(context, state2, type3.instanceType), ConstructorOptions(type3)) : IsDeferred(type3) ? InstantiateDeferred(context, state2, type3.action, type3.parameters, type3.options) : IsFunction3(type3) ? _Function_(InstantiateTypes(context, state2, type3.parameters), InstantiateType(context, state2, type3.returnType), FunctionOptions(type3)) : IsIntersect(type3) ? Intersect(InstantiateTypes(context, state2, type3.allOf), IntersectOptions(type3)) : IsIterator3(type3) ? Iterator(InstantiateType(context, state2, type3.iteratorItems), IteratorOptions(type3)) : IsObject3(type3) ? _Object_(InstantiateProperties(context, state2, type3.properties), ObjectOptions(type3)) : IsPromise(type3) ? _Promise_(InstantiateType(context, state2, type3.item), PromiseOptions(type3)) : IsRecord(type3) ? RecordFromPattern(RecordPattern(type3), InstantiateType(context, state2, RecordValue(type3))) : IsRest(type3) ? Rest(InstantiateType(context, state2, type3.items)) : IsTuple(type3) ? Tuple(InstantiateElements(context, state2, type3.items), TupleOptions(type3)) : IsUnion(type3) ? Union(InstantiateTypes(context, state2, type3.anyOf), UnionOptions(type3)) : type3;
  const withImmutable = immutable ? Immutable(instantiated) : instantiated;
  const withModifiers = ApplyReadonly(modifiers[1], ApplyOptional(modifiers[2], withImmutable));
  return withModifiers;
}
function Instantiate(context, type3) {
  return InstantiateType(context, { callstack: [] }, type3);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/engine/awaited/instantiate.mjs
function AwaitedOperation(type3) {
  return IsPromise(type3) ? AwaitedOperation(type3.item) : type3;
}
function AwaitedAction(type3, options) {
  const result = CanInstantiate([type3]) ? exports_memory.Update(AwaitedOperation(type3), {}, options) : AwaitedDeferred(type3, options);
  return result;
}
function AwaitedInstantiate(context, state2, type3, options) {
  const instantiatedType = InstantiateType(context, state2, type3);
  return AwaitedAction(instantiatedType, options);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/awaited.mjs
function AwaitedDeferred(type3, options = {}) {
  return Deferred("Awaited", [type3], options);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/type/action/evaluate.mjs
function Evaluate2(type3, options = {}) {
  return EvaluateAction(type3, options);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/check/check.mjs
function Check2(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return Check(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/errors/errors.mjs
function Errors2(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  const [_, errors2] = Errors(context, type3, value);
  return errors2;
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/assert/assert.mjs
class AssertError extends Error {
  constructor(source, value, errors3) {
    super(source);
    Object.defineProperty(this, "cause", {
      value: { source, errors: errors3, value },
      writable: false,
      configurable: false,
      enumerable: false
    });
  }
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_array.mjs
function FromArray7(context, type3, value) {
  if (!exports_guard.IsArray(value))
    return value;
  return value.map((value2) => FromType19(context, type3.items, value2));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_base.mjs
function FromBase(_context2, type3, value) {
  return type3.Clean(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_cyclic.mjs
function FromCyclic6(context, type3, value) {
  return FromType19({ ...context, ...type3.$defs }, Ref2(type3.$ref), value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_intersect.mjs
function EvaluateIntersection(context, type3) {
  const additionalProperties3 = exports_guard.HasPropertyKey(type3, "unevaluatedProperties") ? { additionalProperties: type3.unevaluatedProperties } : {};
  const instantiated = Instantiate(context, type3);
  const evaluated = Evaluate2(instantiated);
  return IsObject3(evaluated) ? Options(evaluated, additionalProperties3) : evaluated;
}
function FromIntersect6(context, type3, value) {
  const evaluated = EvaluateIntersection(context, type3);
  return FromType19(context, evaluated, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/additional.mjs
function GetAdditionalProperties(type3) {
  const additionalProperties3 = exports_guard.HasPropertyKey(type3, "additionalProperties") ? type3.additionalProperties : undefined;
  return additionalProperties3;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_object.mjs
function FromObject10(context, type3, value) {
  if (!exports_guard.IsObject(value) || exports_guard.IsArray(value))
    return value;
  const additionalProperties3 = GetAdditionalProperties(type3);
  for (const key of exports_guard.Keys(value)) {
    if (exports_guard.HasPropertyKey(type3.properties, key)) {
      value[key] = FromType19(context, type3.properties[key], value[key]);
      continue;
    }
    const unknownCheck = exports_guard.IsBoolean(additionalProperties3) && exports_guard.IsEqual(additionalProperties3, true) || IsSchema2(additionalProperties3) && Check2(context, additionalProperties3, value[key]);
    if (unknownCheck) {
      value[key] = FromType19(context, additionalProperties3, value[key]);
      continue;
    }
    delete value[key];
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_record.mjs
function FromRecord2(context, type3, value) {
  if (!exports_guard.IsObject(value))
    return value;
  const additionalProperties3 = GetAdditionalProperties(type3);
  const [recordPattern, recordValue] = [new RegExp(RecordPattern(type3)), RecordValue(type3)];
  for (const key of exports_guard.Keys(value)) {
    if (recordPattern.test(key)) {
      value[key] = FromType19(context, recordValue, value[key]);
      continue;
    }
    const unknownCheck = exports_guard.IsBoolean(additionalProperties3) && exports_guard.IsEqual(additionalProperties3, true) || IsSchema2(additionalProperties3) && Check2(context, additionalProperties3, value[key]);
    if (unknownCheck) {
      value[key] = FromType19(context, additionalProperties3, value[key]);
      continue;
    }
    delete value[key];
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_ref.mjs
function FromRef5(context, type3, value) {
  return exports_guard.HasPropertyKey(context, type3.$ref) ? FromType19(context, context[type3.$ref], value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_tuple.mjs
function FromTuple5(context, schema5, value) {
  if (!exports_guard.IsArray(value))
    return value;
  const length = Math.min(value.length, schema5.items.length);
  for (let index2 = 0;index2 < length; index2++) {
    value[index2] = FromType19(context, schema5.items[index2], value[index2]);
  }
  return exports_guard.IsGreaterThan(value.length, length) ? value.slice(0, length) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clone/clone.mjs
function FromClassInstance(value) {
  return value;
}
function FromObjectInstance(value) {
  const result = {};
  for (const key of exports_guard.Keys(value)) {
    if (exports_guard.IsUnsafePropertyKey(key))
      continue;
    result[key] = Clone2(value[key]);
  }
  for (const key of exports_guard.Symbols(value)) {
    result[key] = Clone2(value[key]);
  }
  return result;
}
function FromObject11(value) {
  return exports_guard.IsClassInstance(value) ? FromClassInstance(value) : FromObjectInstance(value);
}
function FromArray8(value) {
  return value.map((element) => Clone2(element));
}
function FromTypedArray(value) {
  return value.slice();
}
function FromMap(value) {
  return new Map(Clone2([...value.entries()]));
}
function FromSet(value) {
  return new Set(Clone2([...value.values()]));
}
function FromValue4(value) {
  return value;
}
function Clone2(value) {
  return exports_globals.IsTypeArray(value) ? FromTypedArray(value) : exports_globals.IsMap(value) ? FromMap(value) : exports_globals.IsSet(value) ? FromSet(value) : exports_guard.IsArray(value) ? FromArray8(value) : exports_guard.IsObject(value) ? FromObject11(value) : FromValue4(value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/shared/union_priority_sort.mjs
function DeterministicCompare(left, right) {
  return JSON.stringify(left).localeCompare(JSON.stringify(right));
}
function UnionPrioritySort(types3, order = 1) {
  return types3.sort((left, right) => {
    const result = Compare(left, right);
    return (exports_guard.IsEqual(result, "disjoint") ? DeterministicCompare(left, right) : exports_guard.IsEqual(result, "right-inside") ? 1 : exports_guard.IsEqual(result, "left-inside") ? -1 : DeterministicCompare(left, right)) * order;
  });
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_union.mjs
function FromUnion9(context, type3, value) {
  for (const schema5 of UnionPrioritySort(type3.anyOf)) {
    const clean = FromType19(context, schema5, Clone2(value));
    if (Check2(context, schema5, clean))
      return clean;
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/from_type.mjs
function FromType19(context, type3, value) {
  return IsArray3(type3) ? FromArray7(context, type3, value) : IsBase(type3) ? FromBase(context, type3, value) : IsCyclic(type3) ? FromCyclic6(context, type3, value) : IsIntersect(type3) ? FromIntersect6(context, type3, value) : IsObject3(type3) ? FromObject10(context, type3, value) : IsRecord(type3) ? FromRecord2(context, type3, value) : IsRef2(type3) ? FromRef5(context, type3, value) : IsTuple(type3) ? FromTuple5(context, type3, value) : IsUnion(type3) ? FromUnion9(context, type3, value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/clean/clean.mjs
function Clean(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return FromType19(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try.mjs
var exports_try = {};
__export(exports_try, {
  TryUndefined: () => TryUndefined,
  TryString: () => TryString,
  TryNumber: () => TryNumber,
  TryNull: () => TryNull,
  TryBoolean: () => TryBoolean,
  TryBigInt: () => TryBigInt,
  TryArray: () => TryArray,
  Ok: () => Ok,
  IsOk: () => IsOk,
  Fail: () => Fail
});

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_result.mjs
function IsOk(value) {
  return exports_guard.IsObject(value) && exports_guard.HasPropertyKey(value, "value");
}
function Ok(value) {
  return { value };
}
function Fail() {
  return;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_array.mjs
function TryArray(value) {
  return exports_guard.IsArray(value) ? Ok(value) : Ok([value]);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_bigint.mjs
function FromBoolean2(value) {
  return exports_guard.IsEqual(value, true) ? Ok(BigInt(1)) : Ok(BigInt(0));
}
var bigintPattern = /^-?(0|[1-9]\d*)n$/;
var decimalPattern = /^-?(0|[1-9]\d*)\.\d+$/;
var integerPattern = /^-?(0|[1-9]\d*)$/;
function IsStringBigIntLike(value) {
  return bigintPattern.test(value);
}
function IsStringDecimalLike(value) {
  return decimalPattern.test(value);
}
function IsStringIntegerLike(value) {
  return integerPattern.test(value);
}
function FromString2(value) {
  const lowercase2 = value.toLowerCase();
  return IsStringBigIntLike(value) ? Ok(BigInt(value.slice(0, value.length - 1))) : IsStringDecimalLike(value) ? Ok(BigInt(value.split(".")[0])) : IsStringIntegerLike(value) ? Ok(BigInt(value)) : exports_guard.IsEqual(lowercase2, "false") ? Ok(BigInt(0)) : exports_guard.IsEqual(lowercase2, "true") ? Ok(BigInt(1)) : Fail();
}
function TryBigInt(value) {
  return exports_guard.IsBigInt(value) ? Ok(value) : exports_guard.IsBoolean(value) ? FromBoolean2(value) : exports_guard.IsNumber(value) ? Ok(BigInt(Math.trunc(value))) : exports_guard.IsNull(value) ? Ok(BigInt(0)) : exports_guard.IsString(value) ? FromString2(value) : exports_guard.IsUndefined(value) ? Ok(BigInt(0)) : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_boolean.mjs
function FromBigInt2(value) {
  return exports_guard.IsEqual(value, BigInt(0)) ? Ok(false) : exports_guard.IsEqual(value, BigInt(1)) ? Ok(true) : Fail();
}
function FromNumber2(value) {
  return exports_guard.IsEqual(value, 0) ? Ok(false) : exports_guard.IsEqual(value, 1) ? Ok(true) : Fail();
}
function FromString3(value) {
  return exports_guard.IsEqual(value.toLowerCase(), "false") ? Ok(false) : exports_guard.IsEqual(value.toLowerCase(), "true") ? Ok(true) : exports_guard.IsEqual(value, "0") ? Ok(false) : exports_guard.IsEqual(value, "1") ? Ok(true) : Fail();
}
function TryBoolean(value) {
  return exports_guard.IsBigInt(value) ? FromBigInt2(value) : exports_guard.IsBoolean(value) ? Ok(value) : exports_guard.IsNumber(value) ? FromNumber2(value) : exports_guard.IsNull(value) ? Ok(false) : exports_guard.IsString(value) ? FromString3(value) : exports_guard.IsUndefined(value) ? Ok(false) : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_null.mjs
function FromBigInt3(value) {
  return exports_guard.IsEqual(value, BigInt(0)) ? Ok(null) : Fail();
}
function FromBoolean3(value) {
  return exports_guard.IsEqual(value, false) ? Ok(null) : Fail();
}
function FromNumber3(value) {
  return exports_guard.IsEqual(value, 0) ? Ok(null) : Fail();
}
function FromString4(value) {
  const lowercase2 = value.toLowerCase();
  const predicate = exports_guard.IsEqual(lowercase2, "undefined") || exports_guard.IsEqual(lowercase2, "null") || exports_guard.IsEqual(value, "") || exports_guard.IsEqual(value, "0");
  return predicate ? Ok(null) : Fail();
}
function TryNull(value) {
  return exports_guard.IsBigInt(value) ? FromBigInt3(value) : exports_guard.IsBoolean(value) ? FromBoolean3(value) : exports_guard.IsNumber(value) ? FromNumber3(value) : exports_guard.IsNull(value) ? Ok(null) : exports_guard.IsString(value) ? FromString4(value) : exports_guard.IsUndefined(value) ? Ok(null) : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_number.mjs
var maxBigInt = BigInt(Number.MAX_SAFE_INTEGER);
var minBigInt = BigInt(Number.MIN_SAFE_INTEGER);
function FromBigInt4(value) {
  return value <= maxBigInt && value >= minBigInt ? Ok(Number(value)) : Fail();
}
function FromBoolean4(value) {
  return Ok(value ? 1 : 0);
}
function FromString5(value) {
  const coerced = +value;
  if (exports_guard.IsNumber(coerced))
    return Ok(coerced);
  const lowercase2 = value.toLowerCase();
  if (exports_guard.IsEqual(lowercase2, "false"))
    return Ok(0);
  if (exports_guard.IsEqual(lowercase2, "true"))
    return Ok(1);
  const result = TryBigInt(value);
  if (IsOk(result))
    return result.value <= maxBigInt && result.value >= minBigInt ? Ok(Number(result.value)) : Fail();
  return Fail();
}
function TryNumber(value) {
  return exports_guard.IsBigInt(value) ? FromBigInt4(value) : exports_guard.IsBoolean(value) ? FromBoolean4(value) : exports_guard.IsNumber(value) ? Ok(value) : exports_guard.IsNull(value) ? Ok(0) : exports_guard.IsString(value) ? FromString5(value) : exports_guard.IsUndefined(value) ? Ok(0) : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_string.mjs
function TryString(value) {
  return exports_guard.IsBigInt(value) ? Ok(value.toString()) : exports_guard.IsBoolean(value) ? Ok(value.toString()) : exports_guard.IsNumber(value) ? Ok(value.toString()) : exports_guard.IsNull(value) ? Ok("null") : exports_guard.IsString(value) ? Ok(value) : exports_guard.IsUndefined(value) ? Ok("") : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/try/try_undefined.mjs
function FromBigInt5(value) {
  return exports_guard.IsEqual(value, BigInt(0)) ? Ok(undefined) : Fail();
}
function FromBoolean5(value) {
  return exports_guard.IsEqual(value, false) ? Ok(undefined) : Fail();
}
function FromNumber4(value) {
  return exports_guard.IsEqual(value, 0) ? Ok(undefined) : Fail();
}
function FromString6(value) {
  const lowercase2 = value.toLowerCase();
  const predicate = exports_guard.IsEqual(lowercase2, "undefined") || exports_guard.IsEqual(lowercase2, "null") || exports_guard.IsEqual(value, "") || exports_guard.IsEqual(value, "0");
  return predicate ? Ok(undefined) : Fail();
}
function TryUndefined(value) {
  return exports_guard.IsBigInt(value) ? FromBigInt5(value) : exports_guard.IsBoolean(value) ? FromBoolean5(value) : exports_guard.IsNumber(value) ? FromNumber4(value) : exports_guard.IsNull(value) ? Ok(undefined) : exports_guard.IsString(value) ? FromString6(value) : exports_guard.IsUndefined(value) ? Ok(value) : Fail();
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_array.mjs
function FromArray9(context, type3, value) {
  const result = exports_try.TryArray(value);
  return result.value.map((value2) => FromType20(context, type3.items, value2));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_base.mjs
function FromBase2(_context2, type3, value) {
  return type3.Convert(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_bigint.mjs
function FromBigInt6(_context2, _type, value) {
  const result = exports_try.TryBigInt(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_boolean.mjs
function FromBoolean6(_context2, _type, value) {
  const result = exports_try.TryBoolean(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_cyclic.mjs
function FromCyclic7(context, type3, value) {
  return FromType20({ ...context, ...type3.$defs }, Ref2(type3.$ref), value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_union.mjs
function FromUnion10(context, type3, value) {
  const matched = type3.anyOf.some((type4) => Check2(context, type4, value));
  if (matched)
    return value;
  const candidates2 = type3.anyOf.map((type4) => FromType20(context, type4, Clone2(value)));
  const selected = candidates2.find((value2) => Check2(context, type3, value2));
  return exports_guard.IsUndefined(selected) ? value : selected;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_enum.mjs
function FromEnum2(context, type3, value) {
  const union3 = EnumToUnion(type3);
  return FromUnion10(context, union3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_integer.mjs
function FromInteger(_context2, _type, value) {
  const result = exports_try.TryNumber(value);
  return exports_try.IsOk(result) ? Math.trunc(result.value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_intersect.mjs
function FromIntersect7(context, type3, value) {
  const instantiated = Instantiate(context, type3);
  const evaluated = Evaluate2(instantiated);
  return FromType20(context, evaluated, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_literal.mjs
function FromLiteralBigInt(_context2, type3, value) {
  const result = exports_try.TryBigInt(value);
  return exports_try.IsOk(result) && exports_guard.IsEqual(type3.const, result.value) ? result.value : value;
}
function FromLiteralBoolean(_context2, type3, value) {
  const result = exports_try.TryBoolean(value);
  return exports_try.IsOk(result) && exports_guard.IsEqual(type3.const, result.value) ? result.value : value;
}
function FromLiteralNumber(_context2, type3, value) {
  const result = exports_try.TryNumber(value);
  return exports_try.IsOk(result) && exports_guard.IsEqual(type3.const, result.value) ? result.value : value;
}
function FromLiteralString(_context2, type3, value) {
  const result = exports_try.TryString(value);
  return exports_try.IsOk(result) && exports_guard.IsEqual(type3.const, result.value) ? result.value : value;
}
function FromLiteral6(context, type3, value) {
  if (exports_guard.IsEqual(type3.const, value))
    return value;
  return IsLiteralBigInt(type3) ? FromLiteralBigInt(context, type3, value) : IsLiteralBoolean(type3) ? FromLiteralBoolean(context, type3, value) : IsLiteralNumber(type3) ? FromLiteralNumber(context, type3, value) : IsLiteralString(type3) ? FromLiteralString(context, type3, value) : Unreachable();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_null.mjs
function FromNull2(_context2, _type, value) {
  const result = exports_try.TryNull(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_number.mjs
function FromNumber5(_context2, _type, value) {
  const result = exports_try.TryNumber(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_additional.mjs
function FromAdditionalProperties(context, entries, additionalProperties3, value) {
  const keys2 = exports_guard.Keys(value);
  for (const [regexp, _] of entries) {
    for (const key of keys2) {
      if (!regexp.test(key)) {
        value[key] = FromType20(context, additionalProperties3, value[key]);
      }
    }
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/shared/optional_undefined.mjs
function IsOptionalUndefined(property, key, value) {
  return IsOptional(property) && exports_guard.IsUndefined(value[key]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_object.mjs
function FromProperties4(context, type3, value) {
  const entries = exports_guard.EntriesRegExp(type3.properties);
  const keys2 = exports_guard.Keys(value);
  for (const [regexp, property] of entries) {
    for (const key of keys2) {
      if (!regexp.test(key) || IsOptionalUndefined(property, key, value))
        continue;
      value[key] = FromType20(context, property, value[key]);
    }
  }
  return exports_guard.HasPropertyKey(type3, "additionalProperties") && exports_guard.IsObject(type3.additionalProperties) ? FromAdditionalProperties(context, entries, type3.additionalProperties, value) : value;
}
function FromObject12(context, type3, value) {
  return exports_guard.IsObjectNotArray(value) ? FromProperties4(context, type3, value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_record.mjs
function FromPatternProperties(context, type3, value) {
  const entries = exports_guard.EntriesRegExp(type3.patternProperties);
  const keys2 = exports_guard.Keys(value);
  for (const [regexp, schema5] of entries) {
    for (const key of keys2) {
      if (regexp.test(key)) {
        value[key] = FromType20(context, schema5, value[key]);
      }
    }
  }
  return exports_guard.HasPropertyKey(type3, "additionalProperties") && exports_guard.IsObject(type3.additionalProperties) ? FromAdditionalProperties(context, entries, type3.additionalProperties, value) : value;
}
function FromRecord3(context, type3, value) {
  return exports_guard.IsObjectNotArray(value) ? FromPatternProperties(context, type3, value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_ref.mjs
function FromRef6(context, type3, value) {
  return exports_guard.HasPropertyKey(context, type3.$ref) ? FromType20(context, context[type3.$ref], value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_string.mjs
function FromString7(_context2, _type, value) {
  const result = exports_try.TryString(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_template_literal.mjs
function FromTemplateLiteral4(context, type3, value) {
  const decoded = TemplateLiteralDecode(type3.pattern);
  return FromType20(context, decoded, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_tuple.mjs
function FromTuple6(context, type3, value) {
  if (!exports_guard.IsArray(value))
    return value;
  for (let index2 = 0;index2 < Math.min(type3.items.length, value.length); index2++) {
    value[index2] = FromType20(context, type3.items[index2], value[index2]);
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_undefined.mjs
function FromUndefined2(_context2, _type, value) {
  const result = exports_try.TryUndefined(value);
  return exports_try.IsOk(result) ? result.value : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_void.mjs
function FromVoid(_context2, _type, value) {
  const result = exports_try.TryUndefined(value);
  return exports_try.IsOk(result) ? undefined : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/from_type.mjs
function FromType20(context, type3, value) {
  return IsArray3(type3) ? FromArray9(context, type3, value) : IsBase(type3) ? FromBase2(context, type3, value) : IsBigInt3(type3) ? FromBigInt6(context, type3, value) : IsBoolean4(type3) ? FromBoolean6(context, type3, value) : IsCyclic(type3) ? FromCyclic7(context, type3, value) : IsEnum2(type3) ? FromEnum2(context, type3, value) : IsInteger3(type3) ? FromInteger(context, type3, value) : IsIntersect(type3) ? FromIntersect7(context, type3, value) : IsLiteral(type3) ? FromLiteral6(context, type3, value) : IsNull3(type3) ? FromNull2(context, type3, value) : IsNumber4(type3) ? FromNumber5(context, type3, value) : IsObject3(type3) ? FromObject12(context, type3, value) : IsRecord(type3) ? FromRecord3(context, type3, value) : IsRef2(type3) ? FromRef6(context, type3, value) : IsString4(type3) ? FromString7(context, type3, value) : IsTemplateLiteral(type3) ? FromTemplateLiteral4(context, type3, value) : IsTuple(type3) ? FromTuple6(context, type3, value) : IsUndefined3(type3) ? FromUndefined2(context, type3, value) : IsUnion(type3) ? FromUnion10(context, type3, value) : IsVoid(type3) ? FromVoid(context, type3, value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/convert/convert.mjs
function Convert(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return FromType20(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_array.mjs
function FromArray10(context, type3, value) {
  if (!exports_guard.IsArray(value))
    return value;
  for (let i = 0;i < value.length; i++) {
    value[i] = FromType21(context, type3.items, value[i]);
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_base.mjs
function FromBase3(context, type3, value) {
  return type3.Default(value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_cyclic.mjs
function FromCyclic8(context, type3, value) {
  return FromType21({ ...context, ...type3.$defs }, Ref2(type3.$ref), value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_default.mjs
function FromDefault(type3, value) {
  if (!exports_guard.IsUndefined(value))
    return value;
  return exports_guard.IsFunction(type3.default) ? type3.default() : Clone2(type3.default);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_intersect.mjs
function FromIntersect8(context, type3, value) {
  const instantiated = Instantiate(context, type3);
  const evaluated = Evaluate2(instantiated);
  return FromType21(context, evaluated, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_object.mjs
function FromObject13(context, type3, value) {
  if (!exports_guard.IsObject(value))
    return value;
  const knownPropertyKeys = exports_guard.Keys(type3.properties);
  for (const key of knownPropertyKeys) {
    const propertyValue = FromType21(context, type3.properties[key], value[key]);
    const isUnassignableUndefined = exports_guard.IsUndefined(propertyValue) && (IsOptional(type3.properties[key]) || !exports_guard.HasPropertyKey(type3.properties[key], "default"));
    if (isUnassignableUndefined)
      continue;
    value[key] = FromType21(context, type3.properties[key], value[key]);
  }
  if (!IsAdditionalProperties(type3) || exports_guard.IsBoolean(type3.additionalProperties))
    return value;
  for (const key of exports_guard.Keys(value)) {
    if (knownPropertyKeys.includes(key))
      continue;
    value[key] = FromType21(context, type3.additionalProperties, value[key]);
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_record.mjs
function FromRecord4(context, type3, value) {
  if (!exports_guard.IsObject(value))
    return value;
  const [recordKey, recordValue] = [new RegExp(RecordPattern(type3)), RecordValue(type3)];
  for (const key of exports_guard.Keys(value)) {
    if (!(recordKey.test(key) && IsDefault(recordValue)))
      continue;
    value[key] = FromType21(context, recordValue, value[key]);
  }
  if (!IsAdditionalProperties(type3))
    return value;
  for (const key of exports_guard.Keys(value)) {
    if (recordKey.test(key))
      continue;
    value[key] = FromType21(context, type3.additionalProperties, value[key]);
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_ref.mjs
function FromRef7(context, type3, value) {
  return exports_guard.HasPropertyKey(context, type3.$ref) ? FromType21(context, context[type3.$ref], value) : value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_tuple.mjs
function FromTuple7(context, schema5, value) {
  if (!exports_guard.IsArray(value))
    return value;
  const [items3, max] = [schema5.items, Math.max(schema5.items.length, value.length)];
  for (let i = 0;i < max; i++) {
    if (i < items3.length)
      value[i] = FromType21(context, items3[i], value[i]);
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_union.mjs
function FromUnion11(context, schema5, value) {
  for (const inner of schema5.anyOf) {
    const result = FromType21(context, inner, Clone2(value));
    if (Check2(context, inner, result)) {
      return result;
    }
  }
  return value;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/from_type.mjs
function FromType21(context, type3, value) {
  const defaulted = IsDefault(type3) ? FromDefault(type3, value) : value;
  return IsArray3(type3) ? FromArray10(context, type3, defaulted) : IsBase(type3) ? FromBase3(context, type3, defaulted) : IsCyclic(type3) ? FromCyclic8(context, type3, defaulted) : IsIntersect(type3) ? FromIntersect8(context, type3, defaulted) : IsObject3(type3) ? FromObject13(context, type3, defaulted) : IsRecord(type3) ? FromRecord4(context, type3, defaulted) : IsRef2(type3) ? FromRef7(context, type3, defaulted) : IsTuple(type3) ? FromTuple7(context, type3, defaulted) : IsUnion(type3) ? FromUnion11(context, type3, defaulted) : defaulted;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/default/default.mjs
function Default(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return FromType21(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/pipeline/pipeline.mjs
function Pipeline(pipeline) {
  return (...args) => {
    const [context, type3, value] = exports_arguments.Match(args, {
      3: (context2, type4, value2) => [context2, type4, value2],
      2: (type4, value2) => [{}, type4, value2]
    });
    return pipeline.reduce((result, func) => func(context, type3, result), value);
  };
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/callback.mjs
function Decode2(_context2, type3, value) {
  return type3["~codec"].decode(value);
}
function Encode(_context2, type3, value) {
  return type3["~codec"].encode(value);
}
function Callback(direction, context, type3, value) {
  if (!IsCodec(type3))
    return value;
  return exports_guard.IsEqual(direction, "Decode") ? Decode2(context, type3, value) : Encode(context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_array.mjs
function Decode3(direction, context, type3, value) {
  if (!exports_guard.IsArray(value))
    return Unreachable();
  for (let i = 0;i < value.length; i++) {
    value[i] = FromType22(direction, context, type3.items, value[i]);
  }
  return Callback(direction, context, type3, value);
}
function Encode2(direction, context, type3, value) {
  const exterior = Callback(direction, context, type3, value);
  if (!exports_guard.IsArray(exterior))
    return exterior;
  for (let i = 0;i < exterior.length; i++) {
    exterior[i] = FromType22(direction, context, type3.items, exterior[i]);
  }
  return exterior;
}
function FromArray11(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode3(direction, context, type3, value) : Encode2(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_cyclic.mjs
function FromCyclic9(direction, context, type3, value) {
  value = FromType22(direction, { ...context, ...type3.$defs }, Ref2(type3.$ref), value);
  return Callback(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_intersect.mjs
function MergeInteriors(interiors) {
  return interiors.reduce((results, interior) => ({ ...results, ...interior }), {});
}
function NonMatchingInterior(value, interiors) {
  for (const interior of interiors)
    if (!exports_guard.IsDeepEqual(value, interior))
      return interior;
  return value;
}
function Decode4(direction, context, type3, value) {
  if (exports_guard.IsEqual(type3.allOf.length, 0))
    return Callback(direction, context, type3, value);
  const interiors = type3.allOf.map((schema5) => FromType22(direction, context, schema5, Clean(schema5, Clone2(value))));
  const structural = interiors.every((result) => exports_guard.IsObject(result));
  const exterior = structural ? MergeInteriors(interiors) : NonMatchingInterior(value, interiors);
  return Callback(direction, context, type3, exterior);
}
function Encode3(direction, context, type3, value) {
  if (exports_guard.IsEqual(type3.allOf.length, 0))
    return Callback(direction, context, type3, value);
  const exterior = Callback(direction, context, type3, value);
  const interiors = type3.allOf.map((schema5) => FromType22(direction, context, schema5, Clean(schema5, Clone2(exterior))));
  const structural = interiors.every((result) => exports_guard.IsObject(result));
  if (structural)
    return MergeInteriors(interiors);
  return NonMatchingInterior(exterior, interiors);
}
function FromIntersect9(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode4(direction, context, type3, value) : Encode3(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_object.mjs
function Decode5(direction, context, type3, value) {
  if (!exports_guard.IsObjectNotArray(value))
    return Unreachable();
  for (const key of exports_guard.Keys(type3.properties)) {
    if (!exports_guard.HasPropertyKey(value, key) || IsOptionalUndefined(type3.properties[key], key, value))
      continue;
    value[key] = FromType22(direction, context, type3.properties[key], value[key]);
  }
  return Callback(direction, context, type3, value);
}
function Encode4(direction, context, type3, value) {
  const exterior = Callback(direction, context, type3, value);
  if (!exports_guard.IsObjectNotArray(exterior))
    return exterior;
  for (const key of exports_guard.Keys(type3.properties)) {
    if (!exports_guard.HasPropertyKey(exterior, key) || IsOptionalUndefined(type3.properties[key], key, exterior))
      continue;
    exterior[key] = FromType22(direction, context, type3.properties[key], exterior[key]);
  }
  return exterior;
}
function FromObject14(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode5(direction, context, type3, value) : Encode4(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_record.mjs
function Decode6(direction, context, type3, value) {
  if (!exports_guard.IsObjectNotArray(value))
    return Unreachable();
  const regexp = new RegExp(RecordPattern(type3));
  for (const key of exports_guard.Keys(value)) {
    if (!regexp.test(key))
      Unreachable();
    value[key] = FromType22(direction, context, RecordValue(type3), value[key]);
  }
  return Callback(direction, context, type3, value);
}
function Encode5(direction, context, type3, value) {
  const exterior = Callback(direction, context, type3, value);
  if (!exports_guard.IsObjectNotArray(exterior))
    return exterior;
  const regexp = new RegExp(RecordPattern(type3));
  for (const key of exports_guard.Keys(exterior)) {
    if (!regexp.test(key))
      continue;
    exterior[key] = FromType22(direction, context, RecordValue(type3), exterior[key]);
  }
  return exterior;
}
function FromRecord5(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode6(direction, context, type3, value) : Encode5(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_ref.mjs
function ResolveRef(direction, context, type3, value) {
  return exports_guard.HasPropertyKey(context, type3.$ref) ? FromType22(direction, context, context[type3.$ref], value) : value;
}
function FromRef8(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Callback(direction, context, type3, ResolveRef(direction, context, type3, value)) : ResolveRef(direction, context, type3, Callback(direction, context, type3, value));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_tuple.mjs
function Decode7(direction, context, type3, value) {
  if (!exports_guard.IsArray(value))
    return Unreachable();
  for (let i = 0;i < Math.min(type3.items.length, value.length); i++) {
    value[i] = FromType22(direction, context, type3.items[i], value[i]);
  }
  return Callback(direction, context, type3, value);
}
function Encode6(direction, context, type3, value) {
  const exterior = Callback(direction, context, type3, value);
  if (!exports_guard.IsArray(exterior))
    return value;
  for (let i = 0;i < Math.min(type3.items.length, exterior.length); i++) {
    exterior[i] = FromType22(direction, context, type3.items[i], exterior[i]);
  }
  return exterior;
}
function FromTuple8(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode7(direction, context, type3, value) : Encode6(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_union.mjs
function Decode8(direction, context, type3, value) {
  for (const schema5 of UnionPrioritySort(type3.anyOf, 1)) {
    if (!Check2(context, schema5, value))
      continue;
    const variant = FromType22(direction, context, schema5, value);
    return Callback(direction, context, type3, variant);
  }
  return value;
}
function Encode7(direction, context, type3, value) {
  const exterior = Callback(direction, context, type3, value);
  for (const schema5 of UnionPrioritySort(type3.anyOf, -1)) {
    const variant = FromType22(direction, context, schema5, Clone2(exterior));
    if (!Check2(context, schema5, variant))
      continue;
    return variant;
  }
  return exterior;
}
function FromUnion12(direction, context, type3, value) {
  return exports_guard.IsEqual(direction, "Decode") ? Decode8(direction, context, type3, value) : Encode7(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/from_type.mjs
function FromType22(direction, context, type3, value) {
  return IsArray3(type3) ? FromArray11(direction, context, type3, value) : IsCyclic(type3) ? FromCyclic9(direction, context, type3, value) : IsIntersect(type3) ? FromIntersect9(direction, context, type3, value) : IsObject3(type3) ? FromObject14(direction, context, type3, value) : IsRecord(type3) ? FromRecord5(direction, context, type3, value) : IsRef2(type3) ? FromRef8(direction, context, type3, value) : IsTuple(type3) ? FromTuple8(direction, context, type3, value) : IsUnion(type3) ? FromUnion12(direction, context, type3, value) : Callback(direction, context, type3, value);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/decode.mjs
class DecodeError extends AssertError {
  constructor(value, errors3) {
    super("Decode", value, errors3);
  }
}
function Assert(context, type3, value) {
  if (!Check2(context, type3, value))
    throw new DecodeError(value, Errors2(context, type3, value));
  return value;
}
function DecodeUnsafe(context, type3, value) {
  return FromType22("Decode", context, type3, value);
}
var Decoder = Pipeline([
  (_context2, _type, value) => Clone2(value),
  (context, type3, value) => Default(context, type3, value),
  (context, type3, value) => Convert(context, type3, value),
  (context, type3, value) => Clean(context, type3, value),
  (context, type3, value) => Assert(context, type3, value),
  (context, type3, value) => DecodeUnsafe(context, type3, value)
]);
function Decode9(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return Decoder(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/encode.mjs
class EncodeError extends AssertError {
  constructor(value, errors3) {
    super("Encode", value, errors3);
  }
}
function Assert2(context, type3, value) {
  if (!Check2(context, type3, value))
    throw new EncodeError(value, Errors2(context, type3, value));
  return value;
}
function EncodeUnsafe(context, type3, value) {
  return FromType22("Encode", context, type3, value);
}
var Encoder = Pipeline([
  (_context2, _type, value) => Clone2(value),
  (context, type3, value) => EncodeUnsafe(context, type3, value),
  (context, type3, value) => Default(context, type3, value),
  (context, type3, value) => Convert(context, type3, value),
  (context, type3, value) => Clean(context, type3, value),
  (context, type3, value) => Assert2(context, type3, value)
]);
function Encode8(...args) {
  const [context, type3, value] = exports_arguments.Match(args, {
    3: (context2, type4, value2) => [context2, type4, value2],
    2: (type4, value2) => [{}, type4, value2]
  });
  return Encoder(context, type3, value);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/codec/has.mjs
function FromArray12(context, type3) {
  return IsCodec(type3) || FromType23(context, type3.items);
}
function FromCyclic10(context, type3) {
  return IsCodec(type3) || FromRef9({ ...context, ...type3.$defs }, Ref2(type3.$ref));
}
function FromIntersect10(context, type3) {
  return IsCodec(type3) || type3.allOf.some((type4) => FromType23(context, type4));
}
function FromObject15(context, type3) {
  return IsCodec(type3) || exports_guard.Keys(type3.properties).some((key) => {
    return FromType23(context, type3.properties[key]);
  });
}
function FromRecord6(context, type3) {
  return IsCodec(type3) || FromType23(context, RecordValue(type3));
}
function FromRef9(context, type3) {
  if (visited.has(type3.$ref))
    return false;
  visited.add(type3.$ref);
  return IsCodec(type3) || exports_guard.HasPropertyKey(context, type3.$ref) && FromType23(context, context[type3.$ref]);
}
function FromTuple9(context, type3) {
  return IsCodec(type3) || type3.items.some((type4) => FromType23(context, type4));
}
function FromUnion13(context, type3) {
  return IsCodec(type3) || type3.anyOf.some((type4) => FromType23(context, type4));
}
function FromType23(context, type3) {
  return IsArray3(type3) ? FromArray12(context, type3) : IsCyclic(type3) ? FromCyclic10(context, type3) : IsIntersect(type3) ? FromIntersect10(context, type3) : IsObject3(type3) ? FromObject15(context, type3) : IsRecord(type3) ? FromRecord6(context, type3) : IsRef2(type3) ? FromRef9(context, type3) : IsTuple(type3) ? FromTuple9(context, type3) : IsUnion(type3) ? FromUnion13(context, type3) : IsCodec(type3);
}
var visited = new Set;
function HasCodec(...args) {
  const [context, type3] = exports_arguments.Match(args, {
    2: (context2, type4) => [context2, type4],
    1: (type4) => [{}, type4]
  });
  visited.clear();
  return FromType23(context, type3);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/error.mjs
class CreateError extends Error {
  constructor(type3, message) {
    super(message);
    this.type = type3;
  }
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_default.mjs
function FromDefault2(_context2, schema5) {
  return exports_guard.IsFunction(schema5.default) ? schema5.default(schema5) : exports_guard.IsObject(schema5.default) ? Clone2(schema5.default) : schema5.default;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_array.mjs
function FromArray13(context, type3) {
  if (IsUniqueItems(type3) && !IsDefault(type3))
    throw new CreateError(type3, "Arrays with uniqueItems constraints must specify a default annotation");
  const length = IsMinItems(type3) ? type3.minItems : 0;
  return Array.from({ length }, () => FromType24(context, type3.items));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_async_iterator.mjs
async function* CreateAsyncIterator() {}
function FromAsyncIterator(_context2, _type) {
  return CreateAsyncIterator();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_base.mjs
function FromBase4(_context2, type3) {
  return type3.Create();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_bigint.mjs
function FromBigInt7(_context2, type3) {
  return IsExclusiveMinimum(type3) ? BigInt(type3.exclusiveMinimum) + BigInt(1) : IsMinimum(type3) ? BigInt(type3.minimum) : BigInt(0);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_boolean.mjs
function FromBoolean7(_context2, _type) {
  return false;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_constructor.mjs
function FromConstructor2(context, type3) {
  const instanceType = FromType24(context, type3.instanceType);
  return class {
    constructor() {
      Object.assign(this, instanceType);
    }
  };
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_cyclic.mjs
function FromCyclic11(context, type3) {
  return FromType24({ ...context, ...type3.$defs }, Ref2(type3.$ref));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_enum.mjs
function FromEnum3(context, type3) {
  return FromType24(context, EnumToUnion(type3));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_function.mjs
function FromFunction2(context, type3) {
  const returnType = FromType24(context, type3.returnType);
  return () => returnType;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_integer.mjs
function FromInteger2(_context2, type3) {
  return IsExclusiveMinimum(type3) && exports_guard.IsNumber(type3.exclusiveMinimum) ? type3.exclusiveMinimum + 1 : IsMinimum(type3) ? type3.minimum : 0;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_intersect.mjs
function FromIntersect11(context, type3) {
  const instantiated = Instantiate(context, type3);
  const evaluated = Evaluate2(instantiated);
  return FromType24(context, evaluated);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_iterator.mjs
function* CreateIterator() {}
function FromIterator(_context2, _type) {
  return CreateIterator();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_literal.mjs
function FromLiteral7(_context2, type3) {
  return type3.const;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_never.mjs
function FromNever(_context2, type3) {
  throw new CreateError(type3, "Cannot create TNever types");
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_null.mjs
function FromNull3(_context2, _type) {
  return null;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_number.mjs
function FromNumber6(_context2, type3) {
  return IsExclusiveMinimum(type3) && exports_guard.IsNumber(type3.exclusiveMinimum) ? type3.exclusiveMinimum + 1 : IsMinimum(type3) ? type3.minimum : 0;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_object.mjs
function FromObject16(context, type3) {
  const required5 = exports_guard.IsUndefined(type3.required) ? [] : type3.required;
  return required5.reduce((result, key) => {
    return { ...result, [key]: FromType24(context, type3.properties[key]) };
  }, {});
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_promise.mjs
function FromPromise(context, type3) {
  return Promise.resolve(FromType24(context, type3.item));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_record.mjs
function FromRecord7(_context2, type3) {
  if (IsMinProperties(type3) && !IsDefault(type3))
    throw new CreateError(type3, "Record with the minProperties constraint must have a default annotation");
  return {};
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_ref.mjs
function FromRef10(context, type3) {
  return exports_guard.HasPropertyKey(context, type3.$ref) ? FromType24(context, context[type3.$ref]) : (() => {
    throw new CreateError(type3, "Unable to deref Ref");
  })();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_string.mjs
function FromString8(_context2, type3) {
  const needsDefault = (IsPattern(type3) || IsFormat(type3)) && !IsDefault(type3);
  if (needsDefault)
    throw Error("Strings with format or pattern constraints must specify default");
  const minLength3 = IsMinLength4(type3) ? type3.minLength : 0;
  return "".padEnd(minLength3);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_symbol.mjs
function FromSymbol2(_context2, _type) {
  return Symbol();
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_template_literal.mjs
function FromTemplateLiteral5(context, type3) {
  const decoded = TemplateLiteralDecode(type3.pattern);
  if (IsString4(decoded))
    throw new CreateError(type3, "Unable to create TemplateLiteral due to infinite type expansion");
  return FromType24(context, decoded);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_tuple.mjs
function FromTuple10(context, type3) {
  return Array.from({ length: type3.minItems }, (_, i) => FromType24(context, type3.items[i]));
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_undefined.mjs
function FromUndefined3(_context2, _type) {
  return;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_union.mjs
function FromUnion14(context, type3) {
  if (exports_guard.IsEqual(type3.anyOf.length, 0)) {
    throw Error("Unable to create Union with no variants");
  }
  return FromType24(context, type3.anyOf[0]);
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_void.mjs
function FromVoid2(_context2, _type) {
  return;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/from_type.mjs
function FromType24(context, type3) {
  return IsDefault(type3) ? FromDefault2(context, type3) : IsArray3(type3) ? FromArray13(context, type3) : IsAsyncIterator3(type3) ? FromAsyncIterator(context, type3) : IsBase(type3) ? FromBase4(context, type3) : IsBigInt3(type3) ? FromBigInt7(context, type3) : IsBoolean4(type3) ? FromBoolean7(context, type3) : IsConstructor3(type3) ? FromConstructor2(context, type3) : IsCyclic(type3) ? FromCyclic11(context, type3) : IsEnum2(type3) ? FromEnum3(context, type3) : IsFunction3(type3) ? FromFunction2(context, type3) : IsInteger3(type3) ? FromInteger2(context, type3) : IsIntersect(type3) ? FromIntersect11(context, type3) : IsIterator3(type3) ? FromIterator(context, type3) : IsLiteral(type3) ? FromLiteral7(context, type3) : IsNever(type3) ? FromNever(context, type3) : IsNull3(type3) ? FromNull3(context, type3) : IsNumber4(type3) ? FromNumber6(context, type3) : IsObject3(type3) ? FromObject16(context, type3) : IsPromise(type3) ? FromPromise(context, type3) : IsRecord(type3) ? FromRecord7(context, type3) : IsRef2(type3) ? FromRef10(context, type3) : IsString4(type3) ? FromString8(context, type3) : IsSymbol3(type3) ? FromSymbol2(context, type3) : IsTemplateLiteral(type3) ? FromTemplateLiteral5(context, type3) : IsTuple(type3) ? FromTuple10(context, type3) : IsUndefined3(type3) ? FromUndefined3(context, type3) : IsUnion(type3) ? FromUnion14(context, type3) : IsVoid(type3) ? FromVoid2(context, type3) : undefined;
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/value/create/create.mjs
function Create2(...args) {
  const [context, type3] = exports_arguments.Match(args, {
    2: (context2, type4) => [context2, type4],
    1: (type4) => [{}, type4]
  });
  return FromType24(context, type3);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/parse/parse.mjs
class ParseError2 extends AssertError {
  constructor(value, errors3) {
    super("Parse", value, errors3);
  }
}
function Assert3(context, type3, value) {
  if (!Check2(context, type3, value))
    throw new ParseError2(value, Errors2(context, type3, value));
  return value;
}
var Parser = Pipeline([
  (_context2, _type, value) => Clone2(value),
  (context, type3, value) => Default(context, type3, value),
  (context, type3, value) => Convert(context, type3, value),
  (context, type3, value) => Clean(context, type3, value),
  (context, type3, value) => Assert3(context, type3, value)
]);
// ../../../../../../../.micro/npm/node_modules/typebox/build/value/delta/edit.mjs
var Insert = _Object_({
  type: Literal("insert"),
  path: String2(),
  value: Unknown()
});
var Update2 = Object({
  type: Literal("update"),
  path: String2(),
  value: Unknown()
});
var Delete2 = _Object_({
  type: Literal("delete"),
  path: String2()
});
var Edit = Union([Insert, Update2, Delete2]);
// ../../../../../../../.micro/npm/node_modules/typebox/build/compile/validator.mjs
class Validator extends Base {
  constructor(...args) {
    super();
    const matched = exports_arguments.Match(args, {
      3: (hasCodec, buildResult, evaluateResult) => [hasCodec, buildResult, evaluateResult],
      2: (context, type3) => [context, type3]
    });
    if (matched.length === 3 && matched[1] instanceof BuildResult && matched[2] instanceof EvaluateResult) {
      const [hasCodec, buildResult, evaluateResult] = matched;
      this.hasCodec = hasCodec;
      this.buildResult = buildResult;
      this.evaluateResult = evaluateResult;
    } else {
      const [context, type3] = matched;
      this.hasCodec = HasCodec(context, type3);
      this.buildResult = Build(context, type3);
      this.evaluateResult = this.buildResult.Evaluate();
    }
  }
  IsAccelerated() {
    return this.evaluateResult.IsAccelerated();
  }
  Context() {
    return this.buildResult.Context();
  }
  Type() {
    return this.buildResult.Schema();
  }
  Code() {
    return this.evaluateResult.Code();
  }
  Check(value) {
    return this.evaluateResult.Check(value);
  }
  Parse(value) {
    const checked = this.Check(value);
    if (checked)
      return value;
    if (exports_settings.Get().correctiveParse)
      return Parser(this.Context(), this.Type(), value);
    throw new ParseError2(value, this.Errors(value));
  }
  Errors(value) {
    if (this.IsAccelerated() && this.Check(value))
      return [];
    return Errors2(this.Context(), this.Type(), value);
  }
  Clean(value) {
    return Clean(this.Context(), this.Type(), value);
  }
  Convert(value) {
    return Convert(this.Context(), this.Type(), value);
  }
  Create() {
    return Create2(this.Context(), this.Type());
  }
  Default(value) {
    return Default(this.Context(), this.Type(), value);
  }
  Decode(value) {
    const result = this.hasCodec ? Decode9(this.Context(), this.Type(), value) : this.Parse(value);
    return result;
  }
  Encode(value) {
    const result = this.hasCodec ? Encode8(this.Context(), this.Type(), value) : this.Parse(value);
    return result;
  }
  Clone() {
    return new Validator(this.hasCodec, this.buildResult, this.evaluateResult);
  }
}

// ../../../../../../../.micro/npm/node_modules/typebox/build/compile/compile.mjs
function Compile(...args) {
  const [context, type3] = exports_arguments.Match(args, {
    2: (context2, type4) => [context2, type4],
    1: (type4) => [{}, type4]
  });
  return new Validator(context, type3);
}
// ../../../../../../../.micro/npm/node_modules/typebox/build/compile/index.mjs
var compile_default = Compile;
export {
  compile_default as default,
  Validator,
  Compile,
  Code
};
