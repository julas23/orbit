# Plano da Orbit Linux

Status: **em andamento** (2026-10-20; revisto em 2026-10-21 com o
`orbit-linux.txt` do Juliano — seção 10 — e com o repositório apt já no ar,
seção 7; e em 2026-11-01 com as decisões da ISO, do boot, do disco e da
identidade visual — seções 6 e 11).
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
| Sessão padrão | `orbit` (orbit-core + orbit-greet) |
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
| ~~`orbit-common-data`~~ | Desnecessário (2026-10-23): paletas, CSS e traduções vão compilados nos binários (`include_str!` em `orbit-app`) |
| `orbit-defaults` | Feito em 2026-10-23 (`distro/defaults/`): aplicativos padrão (`/usr/share/applications/orbit-mimeapps.list`, gerado dos tipos MIME dos apps; texto no Edit, a web fica com o navegador), o override do GSettings (tema escuro) e os exemplos em `/usr/share/doc/orbit/examples`. A sessão já vem no `orbit-core` |
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
O `orbit-desktop` passa a puxar o `orbit-firewall` e o `packagekit`. Os serviços
de rede e de mídia não são metapacotes: cada um é um interruptor na página
"Serviços" do Orbit-Settings (seção 12).

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

Decidido em 2026-10-21 (seção 10) e revisto em 2026-11-01: **pacotes prontos**
— nada é compilado na máquina de quem instala — e **uma ISO só**, que sobe
num menu com duas escolhas.

### 6.1 A ISO

- **Ferramenta:** `live-build`, com a configuração e os hooks versionados em
  `distro/iso/`. Exige root (chroot, squashfs): roda na VM de build do
  Proxmox (seção 7), nunca numa estação.
- **Offline:** a imagem leva a instalação base e o `orbit-desktop`; a rede só
  serve aos extras (ArtCraft, terceiros).
- **Menu do boot da ISO**, sempre visível, com contagem regressiva (ali a
  escolha é o objetivo), em UEFI e em BIOS legado:
  - **Live** — a sessão completa no orbit-core, com painel, dock e apps,
    lida do pendrive, usuário `orbit` sem senha; nada é gravado no disco
    (o `toram` como padrão segurava o boot por um minuto ou mais num
    pendrive, 2026-11-04). Serve também de teste de hardware antes de instalar.
  - **Install** — o debian-installer com preseed, copiando a própria imagem
    da Live (`--debian-installer live`): offline, e instala exatamente o que
    acabou de ser testado.
  - **Opções avançadas** — Live copiada para a memória (`toram`, o pendrive
    pode sair), Install em modo texto, teste de memória.
- **A Live entra sem senha** por um arquivo que só existe na imagem da Live,
  lido pelo orbit-login e pelo orbit-greet; o sistema instalado nunca o tem.
- **Depois:** o `orbit-install` (Rust + GTK4 com o `orbit-app`, seção 10),
  que detecta o hardware e propõe a instalação, toma o lugar da entrada
  Install, e o debian-installer passa às opções avançadas (6.6).

### 6.2 O que a instalação pergunta e faz

- Usuário, senha, hostname, disco, e a criptografia do `/home` quando ela
  se aplica (6.4). A senha nunca passa por linha de comando nem por log.
- O `late.sh` termina o serviço: UKI e systemd-boot (6.3), subvolumes btrfs
  e snapper (6.4), apt apontando para o repositório Orbit e o Debian,
  orbit-login ligado, UID do dono de um `/home` reaproveitado. Log em
  `/var/log/orbit-install.log`.

### 6.3 Boot do sistema instalado

- **systemd-boot com UKI** (kernel, initramfs e cmdline num `.efi` só, via
  `systemd-ukify`; `layout=uki` no `kernel-install`). Troca a decisão de
  EFISTUB puro: a UKI é a mesma, e o systemd-boot invisível custa dezenas de
  milissegundos e dá a contagem de tentativas, que o EFISTUB não tem.
- **Sempre duas UKIs, nunca mais:** a do kernel atual e a do anterior. O apt
  já protege o kernel mais novo, o anterior e o que está rodando; um hook
  nosso apara o excesso.
- **Contagem de tentativas:** cada UKI nova nasce com 3; se não chegar ao
  fim do boot três vezes, o systemd-boot volta sozinho à anterior
  (`systemd-bless-boot` marca a boa).
- **Menu escondido atrás do Espaço** (`timeout 0`), e a possibilidade fica
  clara de três jeitos:
  1. uma linha discreta no rodapé da imagem da UKI e do Plymouth: *Hold Space
     while starting to choose another kernel* (traduzida; em pt_BR "Segure
     Espaço ao ligar para escolher outro kernel");
  2. no menu de energia do greet, **"Reiniciar no menu de boot"**
     (`systemctl reboot --boot-loader-menu=30`), que funciona mesmo quando o
     fast boot do firmware não acorda o teclado USB;
  3. um **aviso no greet** quando a contagem de tentativas voltou ao kernel
     anterior.
- **ESP de 1 GiB**; `/boot` fica dentro do `@` (6.4), então cada snapshot
  guarda o próprio kernel e initramfs.
- **BIOS legado:** GRUB, com o grub-btrfs no papel do nosso hook de
  snapshots. A Live e a Install dão boot nos dois.
- **Secure Boot desligado** (o kernel do Debian só é aceito via shim); chaves
  próprias da Orbit ficam para depois.
- **Plymouth** no initramfs (`quiet splash`), com o tema da seção 11,
  passando ao orbit-greet sem piscar.

### 6.4 Disco

- **btrfs, snapshots só do sistema.** Subvolumes: `@` (`/`, o único com
  snapshots; `/boot` e `/var/lib/dpkg` dentro dele), `@log`, `@cache`,
  `@tmp`, `@snapshots`, e `@home` quando o `/home` não é partição própria.
  `compress=zstd:1,noatime`. A raiz é montada pelo subvolume padrão (voltar
  a um snapshot é trocar o padrão).
- **snapper** com o gancho do apt (um snapshot antes e um depois de cada
  transação, o que cobre as atualizações do Orbit), limpeza por quantidade
  (10 pares; 5 em discos de até 64 GB), sem snapshot por hora, sem cotas do
  btrfs.
- **Snapshots no menu escondido:** os (até 3) mais recentes cujo kernel é
  uma das duas UKIs, cada um com a UKI do seu kernel, subindo só para
  leitura com uma camada temporária na memória. "**Voltar para este**", no
  greet ou por `snapper rollback`, vale para qualquer snapshot: o hook monta
  uma UKI a partir do kernel de dentro do snapshot e a põe no lugar da
  anterior. Modelo: o `sdbootutil` do openSUSE (GPL, shell).
- **Layout pelo tamanho do disco:**

| Disco | Layout | Criptografia |
|---|---|---|
| Menos de 40 GB | Recusado, com mensagem: 40 GB é o mínimo para um uso decente | — |
| 40 a 120 GB | ESP + **um btrfs só**, com `@` e `@home` | Não oferecida |
| Acima de 120 GB | ESP + sistema (**25% do disco, no mínimo 60 e no máximo 100 GiB**) + **`/home` em partição própria** (btrfs sem snapshots) | LUKS2 opcional no `/home` |

- **LUKS só no `/home`, opcional.** O sistema não é cifrado, então o boot
  não pede senha; o `/home` abre no orbit-greet **com a senha do próprio
  usuário** (uma entrada do LUKS por usuário; trocar a senha troca a
  entrada) e fica aberto até desligar. Com o `/home` cifrado, o orbit-login
  **não** abre a sessão antecipada travada no boot: a sessão só começa
  depois que a senha abre o volume. Uma **chave de recuperação**
  (`systemd-cryptenroll --recovery-key`) é mostrada uma vez na instalação,
  com confirmação de que foi anotada.
  Como ficou (2026-11-02): a pergunta vem no fim da instalação
  (`finish-install.d/05orbit-home`, só com `/home` em partição própria),
  antes de o instalador apagar a senha do debconf; o `late.sh` cifra a
  partição (o `/home` ainda só tem a pasta do usuário novo), e o
  `96orbit-recovery` mostra a chave até a resposta ser "sim". No sistema,
  `home: home` no `login.yaml` (a entrada do `/etc/crypttab`, `noauto`, como
  a linha do `/home` no fstab): com o volume fechado, a sessão do boot é a da
  conta de sistema `orbit-greeter` (só o orbit-greet, sem painel), e a senha
  vai ao orbit-login (`login`), que abre o volume (`--disable-keyring`, para
  a chave ficar na tabela do device mapper), monta o `/home`, abre a sessão
  do usuário destravada e encerra a do greeter. O slot de cada usuário fica
  em `/etc/orbit/home-keyslots`; trocar a senha com o `/home` aberto
  (pam_exec no `common-password`, `orbit-login --home-key`) cria um slot novo
  com a chave do volume e apaga o antigo, e um login com a senha antiga ou a
  chave de recuperação também acerta o slot. Com o `/home` cifrado, o swap em
  disco (4 GB de RAM ou menos) usa uma chave aleatória por boot e os core
  dumps ficam desligados.
- **Swap:** zram em todas as máquinas; swap em disco **só com 4 GB de RAM ou
  menos**, do tamanho da RAM: partição (cifrada com chave nova a cada boot
  quando o `/home` é LUKS) no layout separado, arquivo num subvolume `@swap`
  no layout único. Sem hibernação. `/tmp` na memória, core dumps
  desligados, para nada do `/home` cair em área sem criptografia.
- **`/home` existente nunca é apagado** (regra do Juliano, inegociável). O
  instalador reconhece e reaproveita, sem formatar: partição própria (ext4,
  btrfs, xfs…), volume LVM (o caso do hp: `hp-vg-home`), LUKS (pede a senha
  para abrir), subvolume `@home` do btrfs (recria só os do sistema).
  **`/home` como pasta numa partição única**: apaga tudo menos `/home` e
  instala nessa mesma partição, sem formatar — e por isso **sem btrfs nem
  snapshots** quando ela é ext4 (o instalador avisa antes). Um `/home`
  existente não cifrado deixa a opção de LUKS desabilitada (cifrar no lugar é
  arriscado demais para dados de verdade). "Apagar tudo e instalar do zero"
  é sempre oferecido, como escolha explícita. O usuário novo recebe o **UID
  do dono** da pasta existente. Cada layout é testado em VM (Proxmox).

### 6.5 Primeiro boot e testes

- **Primeiro boot:** o orbit-greet cria o usuário se a instalação foi OEM;
  tour rápido opcional (atalhos, painel, dock).
- **Testes:** cada ISO é instalada numa VM do Proxmox (OVMF) por script, e o
  teste confere que a sessão sobe e os apps abrem. A sessão em si se prova no
  think de verdade: se o orbit-core sobe no virtio-gpu de uma VM não está
  garantido.

### 6.6 O orbit-install: detectar e propor (2026-11-15)

Decidido com o usuário em 2026-11-15. Um instalador gráfico **extremamente
simples**, no espírito do Calamares mas com uma tela só: ele detecta o
hardware, monta a proposta de instalação a partir do que achou e pede só o
que não dá para adivinhar. Em Rust e GTK4 com o `orbit-app`, rodando no
orbit-core da Live, como o resto do Orbit; o Calamares (no Debian, em Qt,
assistente de várias páginas) foi descartado por isso.

**O menu do boot da ISO** passa a ter:

- **Orbit Live** — o essencial rodando da mídia, como hoje (6.1).
- **Instalar o Orbit** — a mesma Live, que abre o `orbit-install` em tela
  cheia (`orbit.install` na linha de comando do kernel) em vez da sessão.
- **Opções avançadas** — a Live na memória (`toram`), o debian-installer de
  hoje (em modo gráfico e texto) enquanto o `orbit-install` não cobrir todos
  os layouts da 6.4, teste de memória.

**1. Detectar.**

- **Discos:** tipo (NVMe, SSD, HD: `lsblk`, `rota`, `tran`), tamanho, o que
  têm (`blkid`: sistemas de arquivos, LVM, LUKS) e se algum guarda um
  `/home` (a lógica do `distro/iso/config/includes.installer/orbit/home.sh`).
  Ficam de fora o pendrive da ISO e as mídias removíveis.
- **Placas de rede:** só as físicas (`/sys/class/net/*/device`), quais têm
  cabo ou Wi-Fi conectado, e o endereço IPv4 que cada uma recebeu na Live:
  privado (RFC 1918), CGNAT (`100.64.0.0/10`, tratado como privado) ou
  público. Um endereço recebido por DHCP **prova que a rede já tem um
  servidor DHCP**: naquela interface o modo Rede nunca é sugerido.
- **Vídeo:** a GPU e o nó de render (a conversão do Jellyfin, a codificação
  do remote server).

**2. Propor.**

| Detectado | Proposta |
|---|---|
| Um disco | Sistema e `/home` nele, pelos layouts da 6.4; sem storage |
| Dois ou mais discos | Sistema no mais rápido (NVMe, depois SSD, depois HD; no empate, o menor que tenha 40 GB); os outros, se vazios, como **storage** |
| Discos de storage | Um: btrfs simples. Dois ou mais: btrfs em RAID1 (duas cópias de cada bloco, com quantos discos houver; RAID5/6 do btrfs evitados, por não serem estáveis). Montados em `/srv` (`@media`, `@shares`), onde ficam o `/srv/media` do Navidrome e do Jellyfin e as pastas compartilhadas |
| Uma placa, IP privado | Desktop; ou servidor da rede de casa, com o dnsmasq em modo **Proxy** |
| Uma placa, IP público | Servidor ligado direto na internet: **sem DHCP**, o Unbound só para o túnel, o WireGuard como ponto central (o caso que resolve o CGNAT das outras máquinas). A tela avisa que a máquina está exposta |
| Duas placas, uma pública e uma privada | Servidor como **roteador da casa**: modo **Rede** só na interface privada, com NAT e encaminhamento (a cadeia `forward` do `orbit-firewall`, trabalho novo da seção 12), e só se a interface privada não tiver recebido endereço de outro DHCP |

- **A finalidade não é decidida sozinha.** Uma pergunta só, "Para que é
  esta máquina? **Desktop** / **Servidor**", já marcada pelo que foi
  detectado (Servidor quando há duas placas de rede ligadas ou discos para
  storage; Desktop no resto). Um PC de jogos com dois discos continua
  podendo ser Desktop.
- **Cada finalidade é um conjunto de serviços já ligados** na instalação
  (seção 12), ajustáveis depois na página "Serviços":
  - *Desktop:* o firewall. Discos extras vazios viram storage local, sem
    compartilhar; uma caixa oferece compartilhá-los.
  - *Servidor:* o firewall, o Fail2Ban, o Unbound, o dnsmasq no modo da
    tabela, o WireGuard e o remote server; com storage, o Samba, o NFS e a
    saúde dos discos (smartmontools, com aviso pelo orbit-notify) e o
    Syncthing. Uma caixa **"Kit multimídia (Navidrome e Jellyfin)"** e outra
    **"Sem sessão gráfica"** (desmarcada).
- **Servidor sem sessão gráfica** (um NAS no armário, uma máquina sem
  monitor): instala o `orbit-base` (seção 12.3) em vez do `orbit-desktop` —
  sem orbit-core, greet, painel nem apps; login no console. Liga o SSH (só
  para a LAN e o túnel, por chave; a do usuário pedida na tela, ou a senha
  só até a primeira chave). Gerenciado pelo `orbitctl services` (no console
  ou por SSH) ou pelo Orbit-Settings de um desktop, pelo túnel (12.2). O
  remote server não é oferecido, porque não há sessão para compartilhar. A
  instalação copia a Live como sempre e tira os pacotes do desktop, offline.
- **Disco com dados nunca é formatado sem escolha explícita.** A regra do
  `/home` da 6.4 continua acima de tudo: um disco com `/home` é mantido e
  usado como `/home`. Para storage só entram direto os discos vazios; um
  disco com dados aparece na tela, desmarcado, e só entra se o usuário
  marcar, com o aviso de que será apagado.

**3. Uma tela só.** No alto, o que foi detectado (a máquina, os discos, a
rede) e a proposta em frases ("vou formatar X / vou manter Y", "o DHCP fica
com o roteador"); no meio, a finalidade e, no servidor, o kit multimídia;
embaixo, usuário, senha, nome da máquina e o botão **Instalar**. Idioma e
teclado vêm da Live; o fuso, da rede. Um **"Alterar…"** abre os discos e a
rede para quem quiser mudar a proposta. A criptografia do `/home` (6.4)
aparece quando se aplica.

**Como instala.** Copia o sistema da Live (o `filesystem.squashfs`, como o
Calamares e o `--debian-installer live` fazem: offline, exatamente o que foi
testado) e roda os passos que hoje vivem no debian-installer — a receita
de partições (`early.sh`), os subvolumes, o systemd-boot e as UKIs, o LUKS e
a chave de recuperação, o `/home` reaproveitado (`late.sh`, `home.sh`,
`finish-install.d/`) —, separados do debian-installer para os dois usarem
os mesmos scripts. Depois, liga os serviços da finalidade pelo
`orbit-services-helper` (seção 12). A parte com root roda num processo à
parte, por `pkexec`, com a interface conversando com ela por comandos
tipados. Log em `/var/log/orbit-install.log`.

**Fora da ISO.** Uma VPS não se instala por ISO: os provedores dão imagens
de nuvem prontas. Ali o caminho é um Debian 13 da nuvem, o repositório do
Orbit, os pacotes `orbit-<serviço>` pelo apt e o `orbitctl services` (a
página "Serviços" no terminal: listar, ligar, desligar, configurar), que
serve também a qualquer servidor sem sessão gráfica.


## 7. Infraestrutura

**Repositório apt refeito na VPS em 2026-11-12:** a LXC `10.0.0.111` (aptly
desde 2026-10-21) morreu em 2026-11-10 e levou a chave
`285F 4964 06AB A347 4D1A 5983 9C2D 444C BEB0 4CC0`, que não tinha cópia. Agora
o aptly roda na VPS da Oracle (Ubuntu 24.04, `/srv/aptly`, configuração em
`/srv/orbit-repo-config`, `orbit-repo-sync.timer` toda noite) e o nginx serve
só a publicação `orbit-public` em https://rationalazimuth.com/orbit. Chave
nova `Orbit Linux Archive`
`33CD 1DA3 80C2 3E38 A916 3BE0 F1FB B3C7 B040 716C` (ed25519, até 2029-11-11,
gerada na VPS, sem senha para o timer assinar). **Cópia da chave privada e do
certificado de revogação fora da VPS** — sem ela, perder o servidor é perder o
repositório de novo. Envio dos pacotes:
`distro/build-debs.sh --upload ubuntu@VPS` (só o incoming `orbit` é do
`ubuntu`). Os endereços `10.0.0.111` abaixo são histórico.

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
| Build da ISO | VM 910 `orbit-build` no Proxmox (Debian 13, 8 núcleos, 16 GB, 120 GB, criada em 2026-11-01), com o apt apontando para a LXC 10.0.0.111 |
| VMs de teste | Proxmox, uma VM descartável por ISO |
| Downloads da ISO | Releases do GitHub + checksum assinado |

## 8. Fases

Amarradas às fases do PLANO-CORE.

### Fase A — pacotes (agora, antes do core)

- **Feito em 2026-10-21:** `[package.metadata.deb]` em cada crate,
  `distro/build-debs.sh` gerando os 15 `.deb` e os metapacotes
  (`orbit-desktop`, `orbit-studio`, `orbit-apps`, `orbit-media`, `orbit-dev`),
  e o repositório da seção 7. Falta: CI, e o hp/think usando os pacotes.
- `orbit-defaults`, `orbit-fonts`, `orbit-ruffle` (feitos 2026-10-23); `orbit-common-data` desnecessário.
- CI (feito 2026-10-23): `.github/workflows/debs.yml` → `distro/ci/build.sh` num contêiner Debian 13; os .deb ficam como artefato. Publicar continua em casa.
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

- **Etapa 1:** a ISO da seção 6 com `live-build` — Live na memória e Install
  pelo debian-installer copiando a imagem da Live, systemd-boot com UKI,
  btrfs com snapshots, LUKS opcional no `/home`, a identidade da seção 11.
  Ordem: configuração do live-build → Live em VM (OVMF) → Live no think →
  Install com preseed e `late.sh` em VM (disco inteiro, `/home`
  reaproveitado, LUKS, disco pequeno) → teste automatizado.
- **Etapa 2:** o `orbit-install` dentro da Live.
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
| Nerd Font | O painel usa glifos Nerd Font, que o Debian não tem: pacote `orbit-fonts` (feito 2026-10-23: CaskaydiaCove do lançamento oficial; Cascadia sob OFL 1.1, glifos MIT) |


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

## 11. Identidade visual (2026-11-01)

| Item | Decisão |
|---|---|
| **Marca** | "Trilhas": um núcleo azul (`#4682b4`), uma trilha aberta em volta (`#6ea8e0`) e uma órbita fechada por fora (`#d8dee9`), com a lua laranja (`#ff9500`) sobre ela. Versões colorida (fundo escuro e claro), de uma cor (UKI, terminal, impressão; a lua com um contorno da cor do fundo) e pequena (traços mais grossos, para 16 a 24 px). Fonte do nome ao lado da marca ainda em aberto |
| **Plymouth** | Tema `script` próprio, leve: fundo preto, a marca no centro; a lua percorre a órbita de fora (sentido anti-horário, 3,6 s por volta) e a trilha de dentro gira no sentido contrário (2,2 s). A imagem `.splash` da UKI é o primeiro quadro, com a marca parada; no fim a marca esmaece até o fundo do greet. A linha do Espaço (6.3) no rodapé |
| **Ícones** | Papirus-Dark (Debian, GPL-3), com as pastas na cor de destaque; os apps Orbit com ícones próprios no mesmo estilo |
| **Cursor** | Breeze, que entra nas dependências do `orbit-desktop` (hoje falta: numa máquina nova o cursor cai no padrão) |
| **Cores** | As duas paletas de `data/palette-*.css`; escuro por padrão; destaque azul-aço, o laranja da lua como secundária; uns 8 tons de destaque para escolher |
| **Janelas** | Como hoje no orbit-core: cantos de 10, borda de 2 na cor de destaque na janela com foco, sombra suave só nas flutuantes. Terceiros: Adwaita-dark no GTK, `adwaita-qt` no Qt |
| **Fontes** | Interface: Inter (Noto Sans de reserva). Monoespaçada: a CaskaydiaCove do `orbit-fonts` |
| **Papel de parede** | Um padrão próprio no tema espacial, o mesmo no greet e na sessão |


## 12. Serviços (2026-11-15)

Status: **em revisão** — proposto em 2026-11-15 e revisto com o usuário
item a item antes de começar. Revistos: o modelo (os serviços como
interruptores), a finalidade escolhida na instalação, a ISO e os nomes,
firewall, DHCP e nomes locais, resolvedor, banimento, mídia, storage, o
servidor sem sessão gráfica. Ficam como estão, a analisar depois: o
WireGuard (CGNAT, Tailscale) e a Alexa.
Acompanhamento em `docs/pending.md` §15.

A ideia: cada serviço de rede, de mídia e de storage é um **interruptor na
página "Serviços" do Orbit-Settings**, em qualquer instalação. Ligar
instala, configura e inicia; desligar para; remover desinstala. A
**finalidade** que o `orbit-install` pergunta (Desktop ou Servidor, 6.6) só
decide quais interruptores já vêm ligados; um servidor é uma máquina Orbit
com serviços ligados.
Tudo vem do Debian 13 ou do pacote oficial do upstream (seção 1: nunca um
fork); o trabalho do Orbit são os pacotes de configuração de cada serviço e
a página que os liga.

### 12.1 Decisões

| Item | Decisão |
|---|---|
| **Modelo** (revisto) | Cada serviço é um pacote `orbit-<serviço>` (depende do pacote do Debian ou do upstream e traz a configuração padrão do Orbit e o arquivo do firewall), instalado e configurado pela página "Serviços". Nenhum metapacote os puxa; não há `orbit-server` |
| **Finalidade** (revisto) | O `orbit-install` pergunta "Desktop / Servidor", já marcado pelo hardware detectado (6.6), e liga na instalação o conjunto de serviços da finalidade: o Desktop só o firewall; o Servidor o firewall, o Fail2Ban, o Unbound, o dnsmasq (no modo que a rede pede), o WireGuard, o remote server, o storage quando há discos para ele, e o kit multimídia se marcado. Depois disso tudo se ajusta na página |
| **ISO e nomes** (revisto) | Uma ISO só; a finalidade é uma pergunta do instalador, não outra imagem. *Orbit Linux* é a distribuição; *estação* continua sendo só o thin client (`orbit-local-station`, boot pela rede) |
| **Storage** (revisto) | Os discos além do do sistema, se vazios, viram storage na instalação: btrfs (RAID1 com dois ou mais) em `/srv` (6.6). Serviços: compartilhar pastas pelo Samba (Windows, macOS, Android) e pelo NFS (Linux), a saúde dos discos (smartmontools, com aviso) e a sincronização pelo **Syncthing** (1.29.5 no Debian 13), que o usuário já usa entre as máquinas. ZFS fora (contrib, DKMS a cada kernel). Snapshots dos dados e backup (restic, borg) fora por enquanto |
| **Sistemas fechados** (revisto) | Nenhum trabalho específico para iOS nem para Windows: o que funcionar com os padrões abertos que o Orbit já usa funciona, sem app, serviço ou teste próprio para eles. Fora do plano: o Möbius Sync (Syncthing pago de terceiros no iOS), o backup do iPhone pelo `libimobiledevice` e o Immich (fotos, mas só por Docker) |
| **Servidor sem sessão gráfica** (revisto) | Permitido: `orbit-base` em vez do `orbit-desktop`, login no console, SSH por chave só para a LAN e o túnel, gerência pelo `orbitctl services` ou pelo Orbit-Settings de um desktop por SSH (6.6, 12.2) |
| **Firewall** (revisto) | nftables puro em toda instalação (`orbit-firewall`, puxado pelo `orbit-desktop`, ligado por padrão; desligá-lo pela página pede confirmação). Entrada bloqueada por padrão. Escolhido sobre o firewalld (zonas por conexão do NetworkManager, API D-Bus, regras com prazo, mas um daemon em Python de uns 40 MB) e o ufw (sem zonas nem API, contornado pelo Docker): nenhum daemon, e a camada em volta (a página, o polkit, desfazer em 30 s) é nossa. O `nftables.conf` não usa o `flush ruleset` do padrão do Debian, que apagaria as tabelas do Docker, do libvirt e de outros: só a tabela `inet orbit` é apagada e recarregada. O OpenSnitch (firewall de saída por aplicativo) fica como opcional, fora da página |
| **DHCP e nomes locais** (revisto) | dnsmasq (`orbit-dnsmasq`), em dois modos. **Proxy** (padrão): o DHCP continua no roteador; o dnsmasq faz só o proxy DHCP e o TFTP do boot pela rede, e não conhece os nomes. **Rede** (por escolha): a máquina assume o DHCP no lugar do roteador, e os nomes saem dos leases, mais as reservas fixas (MAC → IP e nome). Domínio `home.lan`, configurável. Só IPv4 por enquanto (sem RA nem DHCPv6). `bind-dynamic`, só nas interfaces da LAN e do túnel: nunca na pública, e sem brigar pela porta 53 com o dnsmasq do libvirt (`virbr0`) e do hotspot do NetworkManager. Escolhido sobre o Kea (sem proxy DHCP nem TFTP, nomes só por DDNS, grande demais para uma casa) e o DHCP do systemd-networkd (sem nomes nem PXE) |
| **Nomes sem esses serviços** (revisto) | mDNS pelo Avahi (`think.local`), que funciona com qualquer roteador; a máquina usa o DNS que a rede entrega |
| **Resolvedor** (revisto) | Unbound (`orbit-unbound`) é o resolvedor da rede, na porta 53 da LAN e do túnel: recursão própria (direto aos servidores raiz e autoritativos) por padrão, DNSSEC, cache, ACL, limite de taxa, `prefetch`. Encaminhar por DNS-over-TLS (Quad9, Cloudflare) é opção. A zona `home.lan` e o reverso `10.in-addr.arpa` vão ao dnsmasq (`forward-zone` + `domain-insecure`), que no modo Rede responde só por ela em `127.0.0.1:5300` (a 5353 é do mDNS). No modo Proxy o Unbound serve a própria máquina, os clientes do túnel e quem apontar o DNS do roteador para ele, com os nomes fixos em `local-data`. Bloqueio de anúncios e rastreadores por listas RPZ: opção, desligado por padrão, listas escolhidas na página |
| **Banimento** (revisto) | Fail2Ban (`orbit-fail2ban`): atual no Debian 13 (1.1.0), leve, só local, lendo do journald e banindo direto no nftables (`nftables-allports`, na tabela `inet orbit`). Jails: `sshd`, e `orbit-remote` quando o remote server está ligado (os códigos de pareamento errados). Com o WireGuard como única porta pública quase não sobra o que banir — ele descarta em silêncio o que não vem de uma chave conhecida —, então o que conta é o SSH aberto por alguém no roteador, um serviço web exposto e quem tenta adivinhar o pareamento pela LAN |
| **CrowdSec** (revisto) | Fora da página; documentado como opção para quem expõe serviços e quer as listas da comunidade. Só se paga com elas (que exigem enviar à API central os ataques vistos, contra o "sem telemetria") e com várias máquinas; sem elas é um Fail2Ban mais pesado (agente, API e bouncer, uns 100 MB), numa versão velha no Debian (1.4.6; o upstream está na 1.6 ou depois, e as coleções do hub podem não servir) ou vindo do repositório deles. A máquina mais exposta, a VPS do repositório (nginx e SSH públicos), é onde ele faria mais sentido, mas ela roda Ubuntu, fora do Orbit Linux |
| **Acesso de fora** | WireGuard (`orbit-wireguard`), como está; a analisar depois: o CGNAT (sem IP público, não há porta de entrada; a VPS como ponto central resolveria) e o Tailscale, que o usuário já usa. Substitui o passo 5 da latência no `pending.md` §9 (TLS, STUN/TURN, porta TCP pública): só a UDP 51820 fica aberta, as estações e os celulares entram pelo túnel, e a 7311 do remote server nunca fica pública. O pareamento entrega à estação a configuração do túnel (QR para celular) |
| **Mídia** (revisto) | O usuário consome a música e os vídeos dele, de qualquer lugar, nos apps que já usa; sem som sincronizado entre cômodos (Snapcast e o DLNA avulso saíram). **Navidrome** (`orbit-navidrome`) para a música: um binário em Go, SQLite, uns 50 MB, API Subsonic/OpenSubsonic e interface web; clientes Feishin (no `orbit-media`), Symfonium, Tempo e Ultrasonic no Android, Amperfy no iPhone. **Jellyfin** (`orbit-jellyfin`) para os vídeos (e música, para quem preferir): apps para TV e celular, conversão de formato na GPU (`jellyfin-ffmpeg`, VA-API), DLNA por plugin para as TVs que só falam isso; mais pesado (uns 300 MB ou mais). O Feishin fala com os dois |
| **Origem dos pacotes de mídia** | Nenhum dos dois está no Debian. Navidrome: o `.deb` oficial das releases do GitHub (amd64 e arm64), uma linha no `distro/repo/github.list`, como o Feishin. Jellyfin: o repositório apt oficial (`repo.jellyfin.org/debian`, suíte `trixie`, amd64 e arm64), uma linha no `distro/repo/vendors.list`, como o Firefox e o VS Code. Nada compilado nem modificado |
| **Alexa** | A decidir depois. A Alexa só toca o que um *skill* entrega, e quem fala com o skill é a nuvem da Amazon: um skill próprio para Subsonic exige conta de desenvolvedor e um endereço HTTPS público (fura o "só o WireGuard é público"); o "My Media for Alexa" é de terceiros, proprietário e com servidor deles; a Echo como caixa Bluetooth do celular não expõe nada, mas não atende a voz |

### 12.2 A página "Serviços"

Uma lista em grupos; cada linha tem o interruptor, o estado (instalado,
rodando, parado, com erro), as portas que abre e um "Configurar…" com a
página do serviço:

| Grupo | Serviço | Pacote | Configurar… |
|---|---|---|---|
| Segurança | Firewall | `orbit-firewall` (já instalado) | o que cada arquivo abre; liberar uma porta à mão |
| Segurança | Banimento (Fail2Ban) | `orbit-fail2ban` | os endereços banidos, liberar um; endereços de confiança (`ignoreip`) |
| Segurança | Terminal remoto (SSH) | `orbit-ssh` | as chaves aceitas; senha permitida ou não (por padrão só chave) |
| Rede | Acesso de fora (WireGuard) | `orbit-wireguard` | os peers: adicionar (com o QR), remover |
| Rede | Resolvedor (Unbound) | `orbit-unbound` | recursão ou DNS-over-TLS; as listas RPZ |
| Rede | DHCP e nomes locais (dnsmasq) | `orbit-dnsmasq` | o modo (Proxy ou Rede), o domínio, a faixa de IPs, as reservas |
| Acesso remoto | Orbit Remote Server | `orbit-remote-server` | a tela virtual ou o espelho; as estações pareadas |
| Acesso remoto | Estações pela rede (LTSP) | `orbit-ltsp` (o `distro/ltsp/` empacotado) | as imagens (PC, Pi 4), o servidor que mostram |
| Mídia | Música (Navidrome) | `orbit-navidrome` | a pasta, o endereço para os apps (LAN e túnel) |
| Mídia | Vídeos (Jellyfin) | `orbit-jellyfin` | as bibliotecas, o endereço para os apps, o plugin DLNA |
| Storage | Pastas compartilhadas (Samba) | `orbit-samba` | as pastas, quem acessa (os usuários do Orbit, com a senha deles no Samba) |
| Storage | Pastas compartilhadas (NFS) | `orbit-nfs` | as pastas e as redes que as montam |
| Storage | Saúde dos discos | `orbit-smart` | os discos, os testes agendados, o histórico |
| Storage | Sincronização (Syncthing) | `orbit-syncthing` | as pastas, os aparelhos (pelo ID ou QR), a descoberta fora da rede |

- **Instalar e remover:** pelo PackageKit (`packagekit`, puxado pelo
  `orbit-desktop`), com a senha pedida pelo polkit (o orbit-polkit) e o
  progresso na própria linha. Remover mantém os dados (músicas, vídeos,
  chaves do túnel) e diz onde ficaram.
- **Iniciar e parar:** o systemd pela API D-Bus dele, autorizado pelo
  polkit; nenhum `systemctl` com root.
- **Configurar:** a configuração em `/etc` é escrita por um auxiliar
  pequeno (`orbit-services-helper`, por `pkexec`, com uma ação polkit
  própria) que só aceita comandos tipados e validados (`dns-mode proxy`,
  `wg-add-peer NOME`, `media-folder music /srv/media/music`…), nunca
  texto livre para arquivos de root.
- **Dependências entre serviços:** o modo Rede do dnsmasq liga o Unbound
  junto; a jail `orbit-remote` do Fail2Ban só existe com o remote server
  ligado; os peers do WireGuard recebem esta máquina como DNS só com o
  Unbound ligado; as Estações pela rede usam o dnsmasq (modo Proxy). A
  página avisa antes de ligar ou desligar um serviço do qual outro depende.
- **Mudanças que podem trancar a máquina** (firewall, WireGuard, modo do
  dnsmasq): aplicadas e desfeitas sozinhas depois de 30 s sem confirmação,
  como os monitores.
- Um ban novo, um serviço que caiu e uma estação que pareou chegam como
  notificação pelo orbit-notify.
- **Outra máquina:** a página pode gerenciar outra máquina Orbit (um
  servidor sem sessão gráfica, por exemplo) pelo SSH, pelo túnel: ela roda
  lá o `orbitctl services --json` e mostra o resultado como se fosse local.
  Nenhum daemon novo ouvindo na rede; a autenticação é a chave do SSH.
- **`orbitctl services`** é a mesma página no terminal (listar, ligar,
  desligar, configurar), sobre o mesmo auxiliar; funciona sem o orbit-core.

### 12.3 Pacotes

| Pacote | Puxado por | Traz |
|---|---|---|
| `orbit-base` | `orbit-desktop`; sozinho no servidor sem sessão gráfica | `orbit-firewall`, `packagekit`, `orbit-services-helper`, `orbitctl` |
| `orbit-firewall` | `orbit-base` | `nftables`; `/etc/nftables.conf` incluindo `/etc/nftables.d/*.nft`; a regra base |
| `packagekit` | `orbit-base` | o que a página usa para instalar e remover |
| `orbit-services-helper` | `orbit-base` | o auxiliar da configuração e a ação polkit |
| `orbit-ssh` | a página; ligado no servidor sem sessão gráfica | `openssh-server`, só chave por padrão, `15-ssh.nft` |
| `orbit-fail2ban` | a página | `fail2ban`, as jails, o filtro `orbit-remote` |
| `orbit-wireguard` | a página | `wireguard-tools`, a `wg0`, `20-wireguard.nft` |
| `orbit-unbound` | a página | `unbound`, a configuração, `40-dns.nft` |
| `orbit-dnsmasq` | a página | `dnsmasq`, os dois modos, `41-dhcp.nft` |
| `orbit-remote-server` | a página | (o pacote que já existe) e `10-remote.nft` |
| `orbit-ltsp` | a página | `ltsp`, o `distro/ltsp/`, `30-ltsp.nft` |
| `orbit-navidrome` | a página | `navidrome`, a configuração, `50-navidrome.nft` |
| `orbit-jellyfin` | a página | `jellyfin` (com `jellyfin-server`, `jellyfin-web` e o `jellyfin-ffmpeg`), a configuração, `51-jellyfin.nft` |
| `orbit-samba` | a página | `samba`, a configuração, `52-samba.nft` |
| `orbit-nfs` | a página | `nfs-kernel-server`, `/etc/exports.d/orbit.exports`, `53-nfs.nft` |
| `orbit-smart` | a página | `smartmontools`, o aviso pelo orbit-notify |
| `orbit-syncthing` | a página | `syncthing`, o serviço por usuário, `54-syncthing.nft` |

### 12.4 Firewall

A tabela `inet orbit` só tem a cadeia `input` (prioridade `filter`,
política `drop`): aceita o tráfego estabelecido, o loopback e o ICMP/ICMPv6
necessário (vizinhança, PMTU). A cadeia `forward` não é tocada, para não
quebrar o Docker e o libvirt, que têm as próprias tabelas; a máquina só
encaminha se o usuário pedir (rotear a LAN pelo túnel, mais adiante).

Cada serviço abre a própria porta num arquivo do seu pacote, **só para a LAN
e para o túnel** (o conjunto `@lan`: as sub-redes privadas das interfaces,
mais a `wg0`):

| Arquivo | Pacote | Abre |
|---|---|---|
| `10-remote.nft` | `orbit-remote-server` | TCP 7311 e a faixa UDP do WebRTC (hoje aleatória: fixar uma faixa no `webrtcbin`) |
| `15-ssh.nft` | `orbit-ssh` | TCP 22 |
| `20-wireguard.nft` | `orbit-wireguard` | UDP 51820, de qualquer lugar (a única porta pública) |
| `30-ltsp.nft` | `orbit-ltsp` | UDP 69 e 4011 (TFTP, proxy DHCP), NFS TCP 2049 |
| `40-dns.nft` | `orbit-unbound` | UDP/TCP 53 |
| `41-dhcp.nft` | `orbit-dnsmasq` | UDP 67 (o proxy DHCP no modo Proxy, o DHCP no modo Rede) |
| `50-navidrome.nft` | `orbit-navidrome` | TCP 4533 |
| `51-jellyfin.nft` | `orbit-jellyfin` | TCP 8096 e a descoberta UDP 7359; SSDP UDP 1900 se o plugin DLNA for ligado |
| `52-samba.nft` | `orbit-samba` | TCP 445 |
| `53-nfs.nft` | `orbit-nfs` | TCP 2049 (NFSv4 só) |
| `54-syncthing.nft` | `orbit-syncthing` | TCP/UDP 22000 (sincronização), UDP 21027 (descoberta na rede) |
| `60-desktop.nft` | `orbit-desktop` | mDNS UDP 5353 e o que o desktop precisar (KDE Connect e afins, se entrarem) |

### 12.5 Cada serviço

- **WireGuard:** a interface `wg0` criada ao ligar, com a chave da máquina;
  cada estação ou celular é um peer. O pareamento do remote server (o
  código de seis dígitos) passa a entregar também a configuração do túnel;
  para celular, um QR.
- **Unbound + dnsmasq:** o Unbound na porta 53 da `@lan` e da `wg0`
  (`access-control` só para elas), recursivo; `forward-zone` de `home.lan`
  e `10.in-addr.arpa` para `127.0.0.1@5300`. O dnsmasq com `port=0` no modo
  Proxy (sem DNS: só o proxy DHCP e o TFTP) e `port=5300` em `127.0.0.1` no
  modo Rede, com `domain=home.lan`, `expand-hosts` e as reservas. O proxy
  DHCP do LTSP entra no mesmo dnsmasq em vez de um segundo. Os peers do
  WireGuard recebem a máquina como DNS, e resolvem `home.lan` de fora de
  casa.
- **Fail2Ban:** `backend = systemd`, `banaction = nftables-allports` na
  tabela do Orbit (o `orbit-firewall` reserva nela o conjunto dos banidos);
  bans curtos (10 min). A jail `sshd` vem do Debian; a `orbit-remote` é
  nossa: um filtro para a linha que o remote server já escreve a cada
  código de pareamento errado (`remote: <endereço> gave a wrong code (n of
  5)`); depois de cinco erros o servidor já troca o código. Essa linha vai
  hoje para `~/.local/state/orbit/remote.log`, fora do journald: o servidor
  passa a escrever também no journald (com o identificador
  `orbit-remote-server`), para a jail ler como as outras. O contador de
  erros é um só para todos os endereços, então quem erra de propósito força
  trocas do código para todos; o ban do Fail2Ban é o que impede isso. Os
  endereços da `@lan` de confiança ficam em `ignoreip` só se o usuário
  pedir.
- **Navidrome e Jellyfin:** cada um como serviço do sistema, com o usuário
  dele, ouvindo só na `@lan` e na `wg0`; fora de casa, o celular entra pelo
  WireGuard e o app conecta como em casa. As pastas escritas na
  configuração dos dois (`MusicFolder` do Navidrome; as bibliotecas do
  Jellyfin pela API dele). Os serviços precisam ler as pastas: o padrão é
  `/srv/media` (`music/`, `videos/`), com um grupo de leitura; pastas no
  `/home` do usuário (que em algumas máquinas é NFS) só por escolha, com
  ACL. O Jellyfin no grupo `render`, para a conversão na GPU. O primeiro
  usuário de cada um é o do Orbit, criado ao ligar, com uma senha própria
  do serviço (os apps Subsonic não falam PAM).
- **Syncthing:** um serviço por usuário do Orbit (`syncthing@USUÁRIO`), com
  a interface e a API em `127.0.0.1:8384`, que a página usa; no servidor, as
  pastas em `/srv` (`@shares`). Os aparelhos se acham na LAN e pelo túnel;
  a descoberta global e os relays (a infraestrutura pública do Syncthing)
  ficam desligados por padrão, como o relatório de uso: ligá-los é uma
  escolha, para quem sincroniza sem o túnel.
- **Celulares:** Android pelo Syncthing-Fork (o fork da comunidade; o app
  oficial foi descontinuado no fim de 2024), sincronizando as pastas do
  usuário (fotos, downloads, documentos, a mídia dos mensageiros) com o
  servidor pelo túnel, também fora de casa. No servidor, uma pasta por
  aparelho (`/srv/@shares/<usuário>/<aparelho>`), com o versionamento do
  Syncthing ligado, para um arquivo apagado no celular não sumir do
  servidor. É cópia dos arquivos, não do aparelho: dados e configurações
  dos apps o Android não deixa ler sem root.
- **SSH:** `openssh-server` só com chave (`PasswordAuthentication no` depois
  da primeira chave), só na `@lan` e na `wg0`; a jail `sshd` do Fail2Ban
  cobre o que sobrar.

### 12.6 Painel

O módulo `network` mostra o estado do firewall e do túnel; um clique leva à
página "Serviços".

### 12.7 Ordem

1. `orbit-firewall`: em toda instalação, e o que mais protege.
2. A página "Serviços" com o PackageKit e o `orbit-services-helper`, e os
   serviços que já existem: o remote server e as estações (`orbit-ltsp`).
   O `orbit-install` (6.6) vem depois dela, porque liga os serviços da
   finalidade pelo mesmo auxiliar.
3. `orbit-wireguard`, com a configuração entregue no pareamento.
4. `orbit-unbound` e `orbit-dnsmasq`, absorvendo o dnsmasq do LTSP.
5. `orbit-fail2ban`, com a jail do Orbit.
6. `orbit-navidrome` e `orbit-jellyfin`.
7. Storage: `orbit-samba`, `orbit-nfs`, `orbit-smart`, `orbit-syncthing`.
8. O `orbit-base`, o `orbit-ssh`, o `orbitctl services` e a gerência de
   outra máquina pelo SSH: o servidor sem sessão gráfica.
9. O painel.

**Pronto quando:** numa VM instalada pela ISO, cada serviço liga, configura,
desliga e sai pela página; um nmap da LAN só vê as portas dos serviços
ligados, e um de fora só vê a UDP 51820; uma estação em outra rede chega à
máquina pelo túnel e mostra a sessão; o Feishin no desktop e um app Subsonic
no celular, este pelo túnel fora de casa, tocam a música do Navidrome, e um
app do Jellyfin toca um vídeo.

### 12.8 Riscos

- Um ban do Fail2Ban vindo da LAN pode trancar a própria casa (um código de
  pareamento digitado errado cinco vezes numa estação): o ban é curto e a
  página libera o endereço.
- O repositório público só carrega pacotes do Orbit (`public.exclude`):
  para quem instala de fora, o `orbit-navidrome` e o `orbit-jellyfin` não
  achariam o `navidrome` e o `jellyfin`. Os dois são software livre
  (GPL-3 e GPL-2) e podem ser redistribuídos: a decidir se o repositório
  público passa a levá-los (a recomendação), ou se a página acrescenta o
  repositório do próprio Jellyfin e baixa o `.deb` do Navidrome.
- O WebRTC usa portas UDP aleatórias: sem uma faixa fixa no remote server, o
  firewall teria de abrir todo UDP da LAN.
- O auxiliar com root é a peça sensível da página: comandos tipados, cada
  valor validado, nada de caminho ou texto livre que chegue a um arquivo de
  root sem passar por um formato conhecido.
