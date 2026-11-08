CREATE TABLE images (
    id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE file_bindings (
    path BLOB PRIMARY KEY NOT NULL,
    image_id TEXT NOT NULL UNIQUE REFERENCES images(id) ON DELETE CASCADE,
    identity BLOB NOT NULL UNIQUE CHECK(length(identity) = 16),
    fingerprint BLOB NOT NULL CHECK(length(fingerprint) = 40)
);
CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE CHECK(length(name) BETWEEN 1 AND 128)
);
CREATE TABLE image_tags (
    image_id TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY(image_id, tag_id)
);
CREATE INDEX image_tags_by_tag ON image_tags(tag_id, image_id);
CREATE TABLE annotations (
    image_id TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('comment', 'description', 'caption', 'prompt')),
    text TEXT NOT NULL,
    PRIMARY KEY(image_id, kind)
);
