import os
import shutil

src = "tredo"
dst = "."

for item in os.listdir(src):
    s = os.path.join(src, item)
    d = os.path.join(dst, item)
    if os.path.exists(d):
        if os.path.isdir(d):
            shutil.rmtree(d)
        else:
            os.remove(d)
    shutil.move(s, d)

print("Files moved successfully!")
