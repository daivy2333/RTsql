CREATE TABLE "users" ("id" INT PRIMARY KEY, "name" STRING NOT NULL, "score" FLOAT);
INSERT INTO "users" VALUES (1, 'alice', 9.5);
INSERT INTO "users" VALUES (2, 'bob', 7.25);
INSERT INTO "users" VALUES (3, 'carol', 8.0);
CREATE TABLE "audit" ("id" INT PRIMARY KEY, "note" STRING);
INSERT INTO "audit" VALUES (1, 'kept');
CREATE TABLE "events" ("id" INT PRIMARY KEY, "day" DATE, "at" TIMESTAMP, "ok" BOOL);
INSERT INTO "events" VALUES (1, DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05', TRUE);
