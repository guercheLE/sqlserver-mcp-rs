/*
 * Shared :r-included fragment: populates #allowlist_patterns (already created
 * by the including script). Keep in sync with ../eda/allowlist.yaml.
 */
INSERT INTO #allowlist_patterns (pattern) VALUES
    ('dm[_]exec[_]%'),('dm[_]os[_]%'),('dm[_]db[_]%'),('dm[_]tran[_]%'),
    ('dm[_]io[_]%'),('dm[_]resource[_]governor[_]%');
