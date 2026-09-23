/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
/** The permissions a user may be granted */
export type UserPermission =
  /** Grant access to read/create their own API keys */
  | 'ACCESS_API_KEYS'
  /**
   * TODO: Expand permissions for bookclub + smartlist
   * Grant access to the book club feature
   */
  | 'ACCESS_BOOK_CLUB'
  /** Grant access to the kobo sync feature */
  | 'ACCESS_KOBO_SYNC'
  /** Grant access to the koreader sync feature */
  | 'ACCESS_KOREADER_SYNC'
  /** Grant access to access the smart list feature. This includes the ability to create and edit smart lists */
  | 'ACCESS_SMART_LIST'
  /**
   * Grant a device the right to act as a remote worker: open the worker
   * socket, claim `worker_jobs`, and upload their outputs
   */
  | 'ACCESS_WORKER'
  /** Grant user access to change **their own** avatar */
  | 'CHANGE_AVATAR'
  /** Grant user access to change **their own** password */
  | 'CHANGE_PASSWORD'
  /** Grant user access to change **their own** username */
  | 'CHANGE_USERNAME'
  /** Grant access to create a book club (access book club) */
  | 'CREATE_BOOK_CLUB'
  /** Grant access to create a library */
  | 'CREATE_LIBRARY'
  /** Grant access to create a notifier */
  | 'CREATE_NOTIFIER'
  /** Grant access to delete the library (manage library) */
  | 'DELETE_LIBRARY'
  /** Grant access to delete a notifier */
  | 'DELETE_NOTIFIER'
  /** Grant access to download files from a library */
  | 'DOWNLOAD_FILE'
  /** Grant access to edit basic details about the library */
  | 'EDIT_LIBRARY'
  /**
   * Grants access to edit any existing metadata for media/series. This will only
   * be applied to the database-level metadata.
   */
  | 'EDIT_METADATA'
  /** Grant access to edit thumbnails for media/series */
  | 'EDIT_THUMBNAILS'
  /** Grant access to create an emailer */
  | 'EMAILER_CREATE'
  /** Grant access to manage an emailer */
  | 'EMAILER_MANAGE'
  /** Grant access to read any emailers in the system */
  | 'EMAILER_READ'
  /** Grant access to send an arbitrary email, bypassing any registered device requirements */
  | 'EMAIL_ARBITRARY_SEND'
  /** Grant access to send an email */
  | 'EMAIL_SEND'
  /** Grant access to access the file explorer */
  | 'FILE_EXPLORER'
  /** Grant access to manage jobs, like pausing, resuming, deleting, or cancelling them */
  | 'MANAGE_JOBS'
  /** Grant access to manage the library (scan,edit,manage relations) */
  | 'MANAGE_LIBRARY'
  /** Grant access to manage a notifier */
  | 'MANAGE_NOTIFIER'
  /** Grant access to manage the server. This is effectively a step below server owner */
  | 'MANAGE_SERVER'
  /** Grant access to manage users (create,edit,delete) */
  | 'MANAGE_USERS'
  /** Grant access to manage metadata fetch statuses (accept matches, etc) */
  | 'METADATA_FETCH_RECORD_MANAGE'
  /** Grant access to read metadata fetch statuses */
  | 'METADATA_FETCH_RECORD_READ'
  /** Grant access to manage metadata provider configurations (create, update, delete) */
  | 'METADATA_PROVIDER_MANAGE'
  /** Grant access to read metadata provider configurations */
  | 'METADATA_PROVIDER_READ'
  /** Grant access to read jobs */
  | 'READ_JOBS'
  /** Grant access to read notifiers */
  | 'READ_NOTIFIER'
  /** Grant access to read application-level logs, e.g. job logs */
  | 'READ_PERSISTED_LOGS'
  /** Grant access to read system logs */
  | 'READ_SYSTEM_LOGS'
  /**
   * Grant access to read users.
   *
   * Note that this is explicitly for querying users via user-specific endpoints.
   * This would not affect relational queries, such as members in a common book club.
   */
  | 'READ_USERS'
  /** Grant access to scan the library for new files */
  | 'SCAN_LIBRARY'
  /** Grant access to upload files to a library */
  | 'UPLOAD_FILE'
  /**
   * Grants access to write back the database-level metadata for media/series.
   * This should be treated with caution, as technically it would allow for
   * overwriting existing metadata at the file-level
   */
  | 'WRITE_BACK_METADATA';

export type MeQueryVariables = Exact<{ [key: string]: never; }>;


export type MeQuery = { me: { id: string, username: string, isServerOwner: boolean, oidcEmail: string | null, lastLogin: string | null, loginSessionsCount: number, permissions: Array<UserPermission> } };


export const MeDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"Me"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"me"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"isServerOwner"}},{"kind":"Field","name":{"kind":"Name","value":"oidcEmail"}},{"kind":"Field","name":{"kind":"Name","value":"lastLogin"}},{"kind":"Field","name":{"kind":"Name","value":"loginSessionsCount"}},{"kind":"Field","name":{"kind":"Name","value":"permissions"}}]}}]}}]} as unknown as DocumentNode<MeQuery, MeQueryVariables>;