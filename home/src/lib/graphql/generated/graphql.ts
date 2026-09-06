/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
/** The storage a device credential references */
export type DeviceCredentialKind =
  /** `credential_ref` is an `api_keys.short_token` */
  | 'API_KEY'
  /** `credential_ref` is a `liseur_sync_tokens.id` */
  | 'LISEUR_TOKEN'
  /** `credential_ref` is a `sessions.session_id` */
  | 'SESSION';

/**
 * The client family a registered device belongs to. The kind decides which
 * credential is minted for the device and which endpoints it is handed.
 */
export type DeviceKind =
  /** A script or integration using the native API */
  | 'API'
  /** A Kobo eReader using the native Kobo sync protocol */
  | 'KOBO'
  /** Komelia using the Komga-compatible profile */
  | 'KOMELIA'
  /** A KOReader install using the KOReader progress sync protocol */
  | 'KOREADER'
  /** Liseur using the native liseur-sync protocol */
  | 'LISEUR'
  /** Mihon (Tachiyomi) using the Komga-compatible profile */
  | 'MIHON'
  /** A generic OPDS reader */
  | 'OPDS'
  /** A browser session */
  | 'WEB';

/**
 * The lifecycle state of a device-pairing request. `Expired` is derived from
 * `expires_at` for pending rows and persisted lazily once observed, so a row's
 * stored status may still read `Pending` after the deadline; always go through
 * [`Model::effective_status`].
 */
export type DevicePairingStatus =
  | 'APPROVED'
  | 'DENIED'
  | 'EXPIRED'
  | 'PENDING';

/** The wire protocol through which a device credential was exercised */
export type DeviceProtocol =
  | 'API'
  | 'KOBO'
  | 'KOMGA'
  | 'KOREADER'
  | 'LISEUR'
  | 'OPDS';

/**
 * How far back reading statistics reach, counted in logical reading days
 * ending today.
 */
export type ReadingStatsSpan =
  | 'ALL_TIME'
  /** The last 30 days */
  | 'MONTH'
  /** The last 90 days */
  | 'QUARTER'
  /** The last 7 days */
  | 'WEEK'
  /** The last 365 days */
  | 'YEAR';

export type DeviceFieldsFragment = { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null };

export type DeviceCredentialFieldsFragment = { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string };

export type DeviceEndpointFieldsFragment = { label: string, url: string, username: string | null, secretHint: string };

export type DevicesQueryVariables = Exact<{ [key: string]: never; }>;


export type DevicesQuery = { devices: Array<{ id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }> };

export type CreateDeviceMutationVariables = Exact<{
  kind: DeviceKind;
  name?: string | null | undefined;
}>;


export type CreateDeviceMutation = { createDevice: { device: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string }, endpoints: Array<{ label: string, url: string, username: string | null, secretHint: string }> } };

export type RenameDeviceMutationVariables = Exact<{
  id: string;
  name: string;
}>;


export type RenameDeviceMutation = { renameDevice: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type RotateDeviceCredentialMutationVariables = Exact<{
  id: string;
}>;


export type RotateDeviceCredentialMutation = { rotateDeviceCredential: { device: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string }, endpoints: Array<{ label: string, url: string, username: string | null, secretHint: string }> } };

export type RevokeDeviceMutationVariables = Exact<{
  id: string;
}>;


export type RevokeDeviceMutation = { revokeDevice: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type SetDeviceTransformProfileMutationVariables = Exact<{
  id: string;
  profile?: unknown;
}>;


export type SetDeviceTransformProfileMutation = { setDeviceTransformProfile: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type PendingDevicePairingsQueryVariables = Exact<{ [key: string]: never; }>;


export type PendingDevicePairingsQuery = { pendingDevicePairings: Array<{ id: string, kind: DeviceKind, name: string | null, remoteIp: string, status: DevicePairingStatus, failedAttempts: number, credentialIssued: boolean, createdAt: string, expiresAt: string, approvedAt: string | null }> };

export type ApproveDevicePairingMutationVariables = Exact<{
  pairingId: string | number;
  code?: string | null | undefined;
}>;


export type ApproveDevicePairingMutation = { approveDevicePairing: { id: string, status: DevicePairingStatus } };

export type DenyDevicePairingMutationVariables = Exact<{
  pairingId: string | number;
}>;


export type DenyDevicePairingMutation = { denyDevicePairing: { id: string, status: DevicePairingStatus } };

export type DeviceSeenSubscriptionVariables = Exact<{
  deviceId?: string | null | undefined;
}>;


export type DeviceSeenSubscription = { deviceSeen: { deviceId: string, userId: string, protocol: DeviceProtocol } };

export type ReadingStatsQueryVariables = Exact<{
  span: ReadingStatsSpan;
}>;


export type ReadingStatsQuery = { readingStats: { span: ReadingStatsSpan, from: string | null, to: string, sessions: number, minutes: number, pages: number, booksFinished: number, streakDays: number, days: Array<{ date: string, sessions: number, minutes: number, pages: number }>, devices: Array<{ deviceId: string, name: string | null, kind: DeviceKind | null, sessions: number, minutes: number, pages: number }> } };

export type MyLoginActivityQueryVariables = Exact<{
  userId: string | number;
}>;


export type MyLoginActivityQuery = { loginActivityById: Array<{ id: number, ipAddress: string, userAgent: string, authenticationSuccessful: boolean, timestamp: string }> };

export const DeviceFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<DeviceFieldsFragment, unknown>;
export const DeviceCredentialFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}}]} as unknown as DocumentNode<DeviceCredentialFieldsFragment, unknown>;
export const DeviceEndpointFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<DeviceEndpointFieldsFragment, unknown>;
export const DevicesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"Devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<DevicesQuery, DevicesQueryVariables>;
export const CreateDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"CreateDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"kind"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceKind"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"name"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"createDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"kind"},"value":{"kind":"Variable","name":{"kind":"Name","value":"kind"}}},{"kind":"Argument","name":{"kind":"Name","value":"name"},"value":{"kind":"Variable","name":{"kind":"Name","value":"name"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"device"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceCredentialFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"endpoints"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceEndpointFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<CreateDeviceMutation, CreateDeviceMutationVariables>;
export const RenameDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RenameDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"name"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"renameDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"name"},"value":{"kind":"Variable","name":{"kind":"Name","value":"name"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<RenameDeviceMutation, RenameDeviceMutationVariables>;
export const RotateDeviceCredentialDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RotateDeviceCredential"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"rotateDeviceCredential"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"device"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceCredentialFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"endpoints"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceEndpointFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<RotateDeviceCredentialMutation, RotateDeviceCredentialMutationVariables>;
export const RevokeDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RevokeDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"revokeDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<RevokeDeviceMutation, RevokeDeviceMutationVariables>;
export const SetDeviceTransformProfileDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetDeviceTransformProfile"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"profile"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"JSON"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setDeviceTransformProfile"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"profile"},"value":{"kind":"Variable","name":{"kind":"Name","value":"profile"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<SetDeviceTransformProfileMutation, SetDeviceTransformProfileMutationVariables>;
export const PendingDevicePairingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"PendingDevicePairings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"pendingDevicePairings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"remoteIp"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"failedAttempts"}},{"kind":"Field","name":{"kind":"Name","value":"credentialIssued"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"expiresAt"}},{"kind":"Field","name":{"kind":"Name","value":"approvedAt"}}]}}]}}]} as unknown as DocumentNode<PendingDevicePairingsQuery, PendingDevicePairingsQueryVariables>;
export const ApproveDevicePairingDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ApproveDevicePairing"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"code"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"approveDevicePairing"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pairingId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}}},{"kind":"Argument","name":{"kind":"Name","value":"code"},"value":{"kind":"Variable","name":{"kind":"Name","value":"code"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}}]}}]}}]} as unknown as DocumentNode<ApproveDevicePairingMutation, ApproveDevicePairingMutationVariables>;
export const DenyDevicePairingDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"DenyDevicePairing"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"denyDevicePairing"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pairingId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}}]}}]}}]} as unknown as DocumentNode<DenyDevicePairingMutation, DenyDevicePairingMutationVariables>;
export const DeviceSeenDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"subscription","name":{"kind":"Name","value":"DeviceSeen"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceSeen"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"deviceId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"userId"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}}]}}]}}]} as unknown as DocumentNode<DeviceSeenSubscription, DeviceSeenSubscriptionVariables>;
export const ReadingStatsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ReadingStats"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"span"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ReadingStatsSpan"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"readingStats"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"span"},"value":{"kind":"Variable","name":{"kind":"Name","value":"span"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"span"}},{"kind":"Field","name":{"kind":"Name","value":"from"}},{"kind":"Field","name":{"kind":"Name","value":"to"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"booksFinished"}},{"kind":"Field","name":{"kind":"Name","value":"streakDays"}},{"kind":"Field","name":{"kind":"Name","value":"days"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"date"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}}]}},{"kind":"Field","name":{"kind":"Name","value":"devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}}]}}]}}]}}]} as unknown as DocumentNode<ReadingStatsQuery, ReadingStatsQueryVariables>;
export const MyLoginActivityDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"MyLoginActivity"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"userId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"loginActivityById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"userId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"ipAddress"}},{"kind":"Field","name":{"kind":"Name","value":"userAgent"}},{"kind":"Field","name":{"kind":"Name","value":"authenticationSuccessful"}},{"kind":"Field","name":{"kind":"Name","value":"timestamp"}}]}}]}}]} as unknown as DocumentNode<MyLoginActivityQuery, MyLoginActivityQueryVariables>;