CREATE TABLE onboarding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1))
);
INSERT INTO onboarding(singleton) VALUES(1);
PRAGMA user_version=12;
