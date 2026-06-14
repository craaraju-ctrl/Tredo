import sqlite3

conn = sqlite3.connect('tredo_history.db')
cursor = conn.cursor()
cursor.execute("SELECT id, timestamp, level, message, details FROM cot_logs ORDER BY timestamp DESC LIMIT 5;")
rows = cursor.fetchall()
print("Latest 5 COT Logs:")
for r in rows:
    print(f"ID: {r[0]} | TS: {r[1]} | Level: {r[2]}")
    print(f"Msg: {r[3]}")
    if r[4]:
        print(f"Details: {r[4][:150]}...")
    print("-" * 50)
conn.close()
