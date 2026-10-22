# Plano da Orbit Linux

Status: **em andamento** (2026-10-20; revisto em 2026-10-21 com o
`orbit-linux.txt` do Juliano — seção 10 — e com o repositório apt já no ar,
seção 7).
Origem: a ideia de 2026-10-20 de levar o Orbit um passo além do desktop: uma
distribuição própria, com o orbit-core, os apps Orbit e as suítes criativas
modernas (ArtCraft) já integrados. Depende de `docs/PLANO-CORE.md`: a distro só
tem identidade quando o core estiver no uso diário.

## 1. Regras que não mudam

1. **Debian é a base, não um ponto de partida para divergir.** Kernel, glibc,
   Mesa, firmware, segurança e drivers vêm do Debian estável (hoje o 13,
   *trixie*). A Orbit Linux é Debian + um repositório Orbit + padrões. Nada de
   recompilar pacotes do Debian.
2. **Tudo o que é Orbit é pacote `.deb`.** Nenhum script copiando binário para
   `~/.local/bin` em máquina alguma: nem no hp, nem no think, nem na ISO.
3. **Um sistema Debian comum vira Orbit com um `apt install orbit-desktop`.** A
   ISO é só a forma mais curta de chegar a isso, não um caminho à parte.
4. **Desinstalável.** `apt remove orbit-desktop` devolve um Debian funcional;
   os pacotes Orbit não sobrescrevem arquivo de outro pacote (usam `diversions`
   ou arquivos próprios em `/etc/xdg`, `/usr/share/orbit`).
5. **App de terceiro só entra no padrão quando estável para o uso diário**; até
   lá fica num metapacote opcional (seção 5).
6. **Builds reproduzíveis e assinados.** Repositório assinado com chave GPG
   própria; pacotes e ISO gerados por CI a partir de tags do git, nunca da
   máquina de alguém.

## 2. Identidade

| Item | Valor |
|---|---|
| Nome | **Orbit Linux** (conferir conflito de nome/marca antes da primeira ISO pública) |
| Base | Debian 13 *trixie* estável; acompanha a próxima estável quando sair |
| Arquiteturas | amd64 primeiro; arm64 depois (ArtCraft já publica arm64) |
| Sessão padrão | `orbit` (orbit-core + orbit-greet); sway como sessão de resgate até a regra 1.2 do PLANO-CORE |
| Repositório | `https://apt.<domínio>/orbit`, suítes `trixie` (estável) e `trixie-testing` (pré-lançamento) |
| Chave | `orbit-archive-keyring` (pacote com a chave, como o `debian-archive-keyring`) |
| Licença dos pacotes Orbit | GPL-3.0-or-later; terceiros mantêm a deles |

## 3. O que já existe e vira pacote

Hoje o `scripts/install.sh` compila os dez apps com `cargo build --release`,
copia os binários para `~/.local/bin` e os `.desktop` de `data/` para
`~/.local/share/applications`. Isso vira:

| Pacote | Conteúdo |
|---|---|
| `orbit-panel`, `orbit-dock` | Binários + `orbit-session` + padrões em `/etc/xdg/orbit/` |
| `orbit-calc`, `orbit-clip`, `orbit-blink` | Binário + `.desktop` + ícone |
| `orbit-view`, `orbit-read`, `orbit-play`, `orbit-edit`, `orbit-files` | Binário + `.desktop` com os tipos MIME + ícone |
| `orbit-core`, `orbit-greet`, `orbit-login`, `orbitctl` | Existem desde 2026-10-22 (PLANO-CORE §7); `orbit-greet` traz `/etc/pam.d/orbit-greet`, `orbit-login` o serviço de sistema |
| `orbit-common-data` | Paletas e CSS compartilhados (`palette-*.css`, `style.css`), traduções (`data/i18n`) |
| `orbit-defaults` | Aplicativos padrão (`/usr/share/applications/orbit-mimeapps.list`), sessão em `/usr/share/wayland-sessions/`, tema de ícones e GTK, dock e painel iniciais |
| `orbit-archive-keyring` | Chave e o arquivo `.sources` do repositório |

**Ferramenta: `cargo-deb`**, não `dh-cargo`. O `dh-cargo` do Debian exige que
cada crate dependente esteja empacotado no Debian, e gtk4-rs 0.10, glycin 2.1.4,
gstreamer 0.25 e afins não estão. O `cargo-deb` empacota a partir do
`Cargo.lock` (dependências vendorizadas no build), calcula as bibliotecas do
sistema que o binário usa (`depends = "$auto"`, via `dpkg-shlibdeps`: no Debian 13
saem `libgtk-4-1`, `libpoppler-glib8t64`, `libgtksourceview-5-0`, GStreamer…) e
gera o pacote com os metadados de `[package.metadata.deb]` no `Cargo.toml` de
cada crate. O `cargo-deb` não está no apt do Debian 13 (conferido em
2026-10-20): entra no CI por `cargo install cargo-deb --locked`, com versão
fixada.

Dependências que não estão no Debian: **Ruffle** (Flash no orbit-play) vira
pacote `orbit-ruffle` com o binário oficial, opcional (`Suggests`). Os loaders
do glycin continuam vindo do Debian (`glycin-loaders`), como hoje.

## 4. Metapacotes

| Metapacote | Puxa |
|---|---|
| `orbit-desktop` | O essencial: core, greet, painel, dock, apps Orbit, `orbit-defaults`, PipeWire, NetworkManager, portal, fontes, Ghostty/terminal |
| `orbit-studio` | Criação: suíte ArtCraft (seção 5) **e** os clássicos (GIMP, Inkscape, Scribus, LibreOffice) enquanto a ArtCraft for alpha |
| `orbit-dev` | Desenvolvimento: VS Code/VSCodium, toolchain Rust, DBeaver, Git, contêineres |
| `orbit-media` | Música, vídeo e jogos: Feishin, Steam/Proton, codecs |
| `orbit-ai` | orbit-aigent (Whisper/Piper), integração com o gateway de IA |

`orbit-desktop` é o único obrigatório; os outros são escolhidos no instalador.

## 5. ArtCraft no Orbit

Conferido em 2026-10-20: suíte de apps criativos em **Rust**, licença
**Apache-2.0** (compatível), interface em **egui**, desenvolvimento muito ativo
(commits diários), builds Linux oficiais em `.deb`, Flatpak e AppImage para
amd64 e arm64. Site oficial: getartcraft.com; código em github.com/storytold.

| App ArtCraft | Substitui | Estágio (out/2026) | No Orbit |
|---|---|---|---|
| PhotoCraft | GIMP / Photoshop | early alpha | `orbit-studio`, ao lado do GIMP |
| VectorCraft | Inkscape / Illustrator | alpha | `orbit-studio`, ao lado do Inkscape |
| DesignCraft | Scribus / InDesign | pré-alpha | `orbit-studio`, ao lado do Scribus |
| WordCraft | LibreOffice Writer / Word | alpha | `orbit-studio`, ao lado do Writer |
| GridCraft | LibreOffice Calc / Excel | alpha | `orbit-studio`, ao lado do Calc |
| Demais (FigCraft, FilmCraft, PdfCraft, SoundCraft, DeckCraft, CADCraft…) | — | variados | avaliar um a um |
| **CraftFonts** | — | **não encontrado** na busca de 2026-10-20 | confirmar nome/existência |

Como entram:

1. **Pacotes deles, não recompilados.** O repositório Orbit espelha os `.deb`
   oficiais da ArtCraft (ou aponta para o repositório deles, se houver), com
   verificação de assinatura/checksum. Recompilar só se o `.deb` deles não
   rodar no Debian estável.
2. **Integração visual:** egui não segue o tema GTK. Fazer um **tema egui do
   Orbit** (paleta Nordic, fontes, cantos) e propor upstream que eles leiam um
   arquivo de tema ou o `org.freedesktop.appearance` do portal (claro/escuro e
   cor de destaque). Seletor de arquivos pelo portal (`rfd` com backend
   `xdg-portal`) — conferir o que eles usam.
3. **Promoção ao padrão** (regra 1.5): um app ArtCraft vira o padrão do seu
   tipo de arquivo quando (a) chegar a beta, (b) abrir e salvar os seus
   arquivos reais sem perda nos testes, e (c) você usá-lo por duas semanas no
   lugar do clássico. Aí o clássico sai do `orbit-studio` e vai para
   `Suggests`.

## 6. Instalação e ISO

Decidido em 2026-10-21 (seção 10): **instalador próprio** e **pacotes
prontos** — nada é compilado na máquina de quem instala.

- **Ferramenta da ISO:** `live-build` (a mesma do Debian live), com a lista de
  pacotes Orbit e os hooks de configuração versionados em `distro/` no repo.
  A ISO leva os pacotes da instalação base e do `orbit-desktop`: **instala
  sem internet**; a rede só serve aos extras (ArtCraft, apps de terceiros).
- **Live mínima:** só a tela de apresentação e o botão **INSTALL**, rodando
  no próprio orbit-core (o `cage` como reserva em placas onde o core falhar).
- **Instalador `orbit-install`:** Rust + GTK4 com o `orbit-app` (o visual do
  Orbit). Pergunta disco, hostname, usuário e senha, e os metapacotes da seção
  4. O trabalho é um backend privilegiado em etapas, com progresso
  ("etapa 3/7") e o log ao vivo na tela (a caixa de saída do orbit-term),
  gravado em `/var/log/orbit-install.log` no sistema instalado. A senha nunca
  passa por linha de comando nem por log. Swap temporário (zram) durante a
  instalação.
- **Base:** `debootstrap` a partir do repositório Orbit (seção 7), depois os
  metapacotes escolhidos.
- **Boot: EFISTUB com UKI** — kernel, initramfs e cmdline num único `.efi`
  (`systemd-ukify`), sem GRUB:
  - registrado com `efibootmgr` (com `/sys/firmware/efi/efivars` montado no
    chroot) **e** copiado para `EFI/BOOT/BOOTX64.EFI`, o caminho que qualquer
    firmware acha sozinho (um `vmlinuz` solto ali não teria `root=` nem
    initrd: só a UKI funciona assim);
  - **duas entradas, atual e anterior**: sem menu de boot, um kernel quebrado
    deixaria a máquina sem boot;
  - refeita nos hooks do kernel **e** do `update-initramfs` (o initrd muda sem
    o kernel mudar);
  - ESP de 1 GiB; `/boot` separado não é mais preciso;
  - **Secure Boot desligado** (o kernel do Debian só é aceito via shim);
    chaves próprias da Orbit ficam para depois;
  - Plymouth no initramfs (`quiet splash`), passando ao orbit-greet.
- **`/home` existente nunca é apagado** (regra do Juliano, inegociável). O
  instalador reconhece e reaproveita, sem formatar:
  - partição própria (ext4, btrfs, xfs…);
  - volume LVM (o caso do hp: `hp-vg-home`);
  - LUKS (pede a senha para abrir);
  - subvolume `@home` do btrfs (recria só o da raiz);
  - **`/home` como pasta dentro da raiz** (partição única): apaga tudo menos
    `/home` e instala nessa mesma partição, sem formatar.
  A alternativa "apagar tudo e instalar do zero" é sempre oferecida, como
  escolha explícita do usuário.
  O usuário novo recebe o **UID do dono** da pasta existente. Antes de tocar
  no disco, uma tela "vou formatar X / vou manter Y" pede confirmação
  explícita. Cada layout é testado em VM (Proxmox).
- **Live para experimentar:** depois do instalador, se valer a pena.
- **Primeiro boot:** o orbit-greet cria o usuário se a instalação foi OEM; tour
  rápido opcional (atalhos, painel, dock).
- **Testes:** cada ISO de CI é instalada numa VM do Proxmox (Dell T430) por
  script, e o teste confere que a sessão sobe e os apps abrem (mesma ideia dos
  testes headless do PLANO-CORE).

## 7. Infraestrutura

**Repositório apt no ar desde 2026-10-21:** LXC `10.0.0.111` (Debian 13), aptly
+ nginx, chave `Orbit Linux Archive (pre-release)`
`285F 4964 06AB A347 4D1A 5983 9C2D 444C BEB0 4CC0` (ed25519, até 2029, gerada
no próprio servidor — a chave definitiva, mestre offline, vem antes da Fase D).

```
# /etc/apt/sources.list.d/orbit.sources (chave em
# http://10.0.0.111/orbit-archive-keyring.gpg → /usr/share/keyrings/)
Types: deb
URIs: http://10.0.0.111/debian http://10.0.0.111/orbit
Suites: trixie
Components: main
Signed-By: /usr/share/keyrings/orbit-archive-keyring.gpg
```

| Repositório | Conteúdo |
|---|---|
| `/debian` | A parte do Debian 13 que o Orbit usa (`distro/repo/debian.list` + prioridades required/important/standard + o que os pacotes de `/orbit` pedem), **com todas as dependências e recomendações**, já com trixie-updates e trixie-security: 4.217 pacotes, 7,7 GB |
| `/orbit` | Os 15 pacotes do Orbit e os 5 metapacotes (`distro/build-debs.sh`), a ArtCraft (14 `.deb` oficiais do GitHub), os terceiros com `.deb` oficial (`distro/repo/github.list`) e a versão mais nova do VS Code e do Firefox dos repositórios deles (`vendors.list`): 45 pacotes |

`distro/repo/sync.sh` monta e publica tudo (snapshots datados, os 7 últimos
guardados para voltar atrás); roda toda noite (`orbit-repo-sync.timer`).
Testado: um `debootstrap` só desse repositório, e nele os cinco metapacotes,
kernel, firmware, UKI, Plymouth, LUKS/LVM/btrfs e rede — 2.019 pacotes
resolvidos e os 2.097 arquivos baixados sem erro, sem nenhuma outra fonte.
Sem `GITHUB_TOKEN` (em `/etc/orbit-repo.env`) a API do GitHub dá 60
consultas por hora; esgotadas, o que já foi baixado fica.

| Peça | Onde |
|---|---|
| Código | Este repositório (`distro/` para ISO e metapacotes) |
| CI dos pacotes e da ISO | GitHub Actions no começo; runner próprio no Proxmox quando o build pesar |
| Repositório apt | aptly na LXC `10.0.0.111` (rede de casa, HTTP); a VPS da Oracle com HTTPS para o público, na Fase D |
| Chave de assinatura | Gerada offline; subchave de assinatura só no CI (segredo do Actions); chave mestre fora de qualquer máquina ligada à rede |
| VMs de teste | Proxmox, uma VM descartável por ISO |
| Downloads da ISO | Releases do GitHub + checksum assinado |

## 8. Fases

Amarradas às fases do PLANO-CORE.

### Fase A — pacotes (agora, antes do core)

- **Feito em 2026-10-21:** `[package.metadata.deb]` em cada crate,
  `distro/build-debs.sh` gerando os 15 `.deb` e os metapacotes
  (`orbit-desktop`, `orbit-studio`, `orbit-apps`, `orbit-media`, `orbit-dev`),
  e o repositório da seção 7. Falta: CI, e o hp/think usando os pacotes.
- `orbit-common-data`, `orbit-defaults`, `orbit-ruffle`.
- Repositório apt na VPS + `orbit-archive-keyring`; CI publicando a cada tag.
- **Pronto quando:** o hp troca o `install.sh` pelos pacotes sem diferença no
  uso; o **think** instala tudo com `apt install orbit-desktop` (fecha a
  pendência dele); `apt remove` deixa o Debian limpo.

### Fase B — metapacotes e Studio (junto com a Fase 1 do core)

- Metapacotes da seção 4; `orbit-studio` com a ArtCraft espelhada e os
  clássicos.
- Tema egui do Orbit e a conversa upstream com a ArtCraft.
- **Pronto quando:** um Debian 13 recém-instalado numa VM vira o seu desktop
  atual com `apt install orbit-desktop orbit-studio`.

### Fase C — ISO (depois da Fase 2 do core)

- `live-build` + Calamares com a marca Orbit; sessão `orbit` com orbit-greet
  como padrão; sway de resgate.
- Teste automatizado de instalação em VM do Proxmox.
- **Pronto quando:** a ISO instala no think do zero (disco inteiro, LUKS) e ele
  sobe no orbit-greet com tudo funcionando, sem nenhum passo manual.

### Fase D — lançamento público

- Site, documentação, nome/marca conferidos, canal de atualizações estável e de
  teste, política de segurança (como reportar, prazo de correção).
- Imagem arm64.
- **Pronto quando:** alguém fora de casa instala pela ISO e usa sem a sua ajuda.

## 9. Pendências e riscos

| Item | Risco / decisão em aberto |
|---|---|
| Popsicle, Ghostty, AniCraft, craft-fonts | Sem `.deb` oficial: AppImage gerenciada pelo Orbit, ou pacote próprio |
| Nerd Font | O painel usa glifos Nerd Font, que o Debian não tem: pacote `orbit-fonts` (MIT) |


| Item | Risco / decisão em aberto |
|---|---|
| Manutenção | Uma distro é compromisso de longo prazo. A regra 1.1 (só Debian + repositório) existe para que o custo seja o dos pacotes Orbit, não o de um sistema inteiro |
| ArtCraft em alpha | Formato de arquivo e interface podem mudar; por isso ficam opcionais e os clássicos continuam até a promoção (seção 5.3) |
| egui x GTK | Visual diferente dos apps Orbit até existir tema/integração; risco de nunca ficar idêntico |
| Versões dos `.deb` da ArtCraft | As páginas de produto deles mostraram versões diferentes das do rastreador oficial na busca de 2026-10-20; espelhar sempre do lançamento oficial (GitHub releases), não de páginas de terceiros |
| Sites não oficiais | A busca trouxe vários domínios que imitam a ArtCraft (guias, "downloads"); só usar getartcraft.com e github.com/storytold |
| Nome "Orbit Linux" | Verificar se já existe distro, produto ou marca com esse nome antes da Fase D |
| Firmware não livre | Debian 13 já inclui `non-free-firmware` no instalador; a ISO segue o mesmo (o think precisa de `firmware-amd-graphics`) |
| Chave de assinatura | Perder ou vazar a chave obriga a trocar em todas as máquinas; processo de rotação documentado antes da Fase D |
| Steam, VS Code, Chrome | Repositórios de terceiros não livres; entram nos metapacotes opcionais configurando o repositório oficial deles, nunca redistribuídos pelo Orbit |

## 10. O `orbit-linux.txt` (2026-10-21)

O plano do Juliano para a distro: boot por EFISTUB com hooks do kernel,
Plymouth, orbit-greet, orbit-core com o rack inteiro, a ArtCraft e uma lista
de apps; uma ISO live com uma tela e um botão INSTALL; e a regra de nunca
apagar um `/home` existente. Discutido e aceito com estas mudanças:

- **Pacotes prontos em vez de compilar na instalação** (o txt clonava e
  compilava tudo com `cargo build` no chroot): compilar na máquina de quem
  instala leva horas, pode estourar a memória, deixa o sistema pela metade se
  a rede cair, precisa de `curl | sh` no instalador, e cada atualização
  pediria compilar de novo; o ganho de otimizar para o processador é pequeno
  em apps gráficos. Compilar do código pode voltar como modo avançado.
- **Instalador próprio** (como no txt, contra o Calamares), em Rust + GTK4
  com o `orbit-app`, não Iced/Slint.
- **EFISTUB na forma de UKI** (seção 6): a dica do txt de usar
  `BOOTX64.EFI` só funciona assim.
- **ISO com os pacotes dentro** (instala sem rede).
- Correções do txt: o Orbit é `github.com/julas23/orbit` (não
  `storytold/orbit-core`); `chroot -u` não existe; `sway` não entra;
  "DeskCraft" é a **DeckCraft**.
