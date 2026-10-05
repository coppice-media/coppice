import type { IngestSettingValueType } from '$lib/graphql/generated/graphql';

/** The parts of an `IngestSettingDefinition` a settings control renders from. */
export interface SettingDescriptor {
	key: string;
	label: string;
	valueType: IngestSettingValueType;
	required: boolean;
	defaultValue?: unknown;
	description?: string | null;
	minimum?: number | null;
	maximum?: number | null;
	options: string[];
}

/**
 * A control's local value. `null` inherits the server default: the key is
 * left out of the saved settings object, so the server falls back to the
 * descriptor default.
 */
export type SettingDraft = string | boolean | null;

export type SettingCheck = { ok: true; value: unknown } | { ok: false; error: string };

const WHOLE_NUMBER = /^[+-]?\d+$/;

/** The control value for a stored (or default) JSON value. */
export function draftFromValue(definition: SettingDescriptor, value: unknown): string | boolean {
	switch (definition.valueType) {
		case 'BOOLEAN':
			return value === true;
		case 'JSON':
			return value === undefined ? '' : JSON.stringify(value, null, 2);
		default:
			return value === null || value === undefined ? '' : String(value);
	}
}

/** Human-readable range of a numeric setting, e.g. `≥ 1` or `1–10`. */
export function boundsLabel(definition: SettingDescriptor): string | null {
	const { minimum, maximum } = definition;
	if (minimum != null && maximum != null) return `${minimum}–${maximum}`;
	if (minimum != null) return `≥ ${minimum}`;
	if (maximum != null) return `≤ ${maximum}`;
	return null;
}

function checkBounds(definition: SettingDescriptor, value: number): SettingCheck {
	if (definition.minimum != null && value < definition.minimum) {
		return { ok: false, error: `Must be at least ${definition.minimum}.` };
	}
	if (definition.maximum != null && value > definition.maximum) {
		return { ok: false, error: `Must be at most ${definition.maximum}.` };
	}
	return { ok: true, value };
}

/**
 * Validates a control value against its descriptor the way the server's
 * `SettingDefinition::validate` does, and returns the JSON value to save.
 */
export function validateSetting(definition: SettingDescriptor, draft: string | boolean): SettingCheck {
	if (definition.valueType === 'BOOLEAN') {
		return typeof draft === 'boolean' ? { ok: true, value: draft } : { ok: false, error: 'Choose on or off.' };
	}
	const text = typeof draft === 'string' ? draft : String(draft);
	switch (definition.valueType) {
		case 'INTEGER': {
			const trimmed = text.trim();
			if (!WHOLE_NUMBER.test(trimmed)) return { ok: false, error: 'Enter a whole number.' };
			const value = Number(trimmed);
			if (!Number.isSafeInteger(value)) return { ok: false, error: 'The number is too large.' };
			return checkBounds(definition, value);
		}
		case 'NUMBER': {
			const trimmed = text.trim();
			const value = Number(trimmed);
			if (!trimmed || !Number.isFinite(value)) return { ok: false, error: 'Enter a number.' };
			return checkBounds(definition, value);
		}
		case 'ENUM':
			if (!definition.options.includes(text)) {
				return { ok: false, error: `Choose one of ${definition.options.join(', ')}.` };
			}
			return { ok: true, value: text };
		case 'JSON':
			try {
				return { ok: true, value: JSON.parse(text) };
			} catch {
				return { ok: false, error: 'Enter valid JSON.' };
			}
		default:
			if (definition.required && !text.trim()) return { ok: false, error: 'A value is required.' };
			return { ok: true, value: text };
	}
}
