import sqlite3
conn = sqlite3.connect('wem-data/wem.db')
cur = conn.cursor()

print("=== Target blocks ===")
cur.execute("SELECT id, parent_id, block_type, position, status FROM blocks WHERE id IN ('202604181653484161Cu', '20260419115651447lKF')")
for row in cur.fetchall():
    print(row)

print("\n=== Children of parent ===")
cur.execute("SELECT id, parent_id, block_type, position, status FROM blocks WHERE parent_id = '202604181653484161Cu' AND status != 'deleted' ORDER BY position")
for row in cur.fetchall():
    print(row)

print("\n=== Schema ===")
cur.execute("SELECT sql FROM sqlite_master WHERE type='table' AND name='blocks'")
for row in cur.fetchall():
    print(row[0])

conn.close()
