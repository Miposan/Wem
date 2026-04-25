import sqlite3
conn = sqlite3.connect('wem-data/wem.db')
cur = conn.cursor()

print("=== content_type values ===")
cur.execute("SELECT id, content_type FROM blocks WHERE status != 'deleted' LIMIT 10")
for row in cur.fetchall():
    print(row)

print("\n=== Check NOT NULL constraint on content_type ===")
cur.execute("PRAGMA table_info(blocks)")
for row in cur.fetchall():
    if 'content_type' in str(row):
        print(row)

print("\n=== All indexes ===")
cur.execute("SELECT name, sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL")
for row in cur.fetchall():
    print(row[0], ":", row[1])

conn.close()
