import os
import shutil

src = "/Users/varma/Desktop/tredo"
dst = "/Users/varma/Desktop/TREDO"

# List src contents and copy them to dst (avoiding deleting dst/symlink.py during execution)
for item in os.listdir(src):
    if item == "symlink.py":
        continue
    s = os.path.join(src, item)
    d = os.path.join(dst, item)
    if os.path.exists(d):
        if os.path.isdir(d):
            shutil.rmtree(d)
        else:
            os.remove(d)
    if os.path.isdir(s):
        shutil.copytree(s, d, symlinks=True)
    else:
        shutil.copy2(s, d)

print("Copy completed successfully!")
