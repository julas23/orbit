
Módulo      Esforço        Motivo
orbit-clip  baixo a médio  Interface pequena, protocolo de área de transferência do Wayland e sincronização com o XWayland
orbit-blink baixo a médio  Capturador de tela Modos [tela inteira, janela atual, retângulo]
orbit-view  médio          Visualizador de imagem: decodificar, aplicar zoom, navegar pela pasta
orbit-read  médio          TXT e MD são simples; PDF depende de uma biblioteca de renderização
orbit-play  médio a alto   Precisa de GStreamer e lida com muitos formatos
orbit-talk  alto           Integrar Whisper e Piper e cuidar da latência
orbit-files alto           Um gerenciador de arquivos tem muitos casos de dintegrações (YAZI de referência)
orbit-edit  alto           Editor de texto de verdade
orbit-core  muito alto     É o compositor em Smithay, um projeto à parte


orbit-calc  baixo a médio  Múltiplos formatos e finalidades de calculadoras (COMPLETED)
