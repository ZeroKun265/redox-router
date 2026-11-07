# Publishing RedoxRouter to the Arch User Repository (AUR)

No approval is needed! Anyone with an account on [aur.archlinux.org](https://aur.archlinux.org/) can publish an AUR package.

---

## 1. Prerequisites (One-Time Setup)
1. Register an account at [https://aur.archlinux.org/register/](https://aur.archlinux.org/register/)
2. Upload your SSH public key (`~/.ssh/id_ed25519.pub` or `~/.ssh/id_rsa.pub`) to **My Account** on aur.archlinux.org.
3. Test your SSH connection:
   ```bash
   ssh -T aur@aur.archlinux.org
   # Should output: Interactive shell is disabled.
   ```

---

## 2. Publish `redox-router-bin` (Recommended for fast installs)

```bash
# 1. Clone the (initially empty) AUR repo
git clone ssh://aur@aur.archlinux.org/redox-router-bin.git /tmp/redox-router-bin

# 2. Copy the PKGBUILD into the repo
cp extra/aur/redox-router-bin/PKGBUILD /tmp/redox-router-bin/

# 3. Enter directory and update sha256 checksums
cd /tmp/redox-router-bin
updpkgsums   # (part of 'pacman-contrib' package)

# 4. Generate the mandatory .SRCINFO file
makepkg --printsrcinfo > .SRCINFO

# 5. Commit and push to publish!
git add PKGBUILD .SRCINFO
git commit -m "Release 0.2.0"
git push origin master
```

---

## 3. Publish `redox-router` (Source build)

```bash
git clone ssh://aur@aur.archlinux.org/redox-router.git /tmp/redox-router
cp extra/aur/redox-router/PKGBUILD /tmp/redox-router/
cd /tmp/redox-router
updpkgsums
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "Release 0.2.0"
git push origin master
```

Once pushed, anyone on Arch can install it via:
```bash
yay -S redox-router-bin
# or
paru -S redox-router-bin
```

