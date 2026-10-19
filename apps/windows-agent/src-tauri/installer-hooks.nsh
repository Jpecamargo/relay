; Hooks do instalador NSIS (per-machine, roda elevado). $INSTDIR contém o sidecar relay-service.exe.

!macro NSIS_HOOK_PREINSTALL
  ; Update: para o serviço e encerra o helper (mesmo exe, --session-helper) antes de sobrescrever
  ; relay-service.exe em uso. Primeira instalação: os comandos falham e o erro é ignorado.
  nsExec::ExecToLog 'sc.exe stop RelayService'
  Pop $0
  nsExec::ExecToLog 'taskkill.exe /F /IM relay-service.exe'
  Pop $0
  ; Dá tempo ao SCM/Windows de soltar o arquivo.
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Serviço RelayService (SYSTEM) + regras de firewall do Relay. Valem em qualquer perfil (o Windows
  ; costuma marcar Ethernet nova como Pública), mas só para a sub-rede local.
  nsExec::ExecToLog '"$INSTDIR\relay-service.exe" install'
  Pop $0
  ; Rede de segurança: garante o início mesmo se o install não conseguiu (já rodando = ok).
  nsExec::ExecToLog 'sc.exe start RelayService'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Relay"'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Relay" dir=in action=allow protocol=TCP localport=47800,47804 profile=any remoteip=localsubnet'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Relay" dir=in action=allow protocol=UDP localport=47801-47803 profile=any remoteip=localsubnet'
  Pop $0

  ; VB-CABLE (microfone do Mac no PC): pacote oficial da VB-Audio, oferecido só se ainda não estiver
  ; instalado e o usuário não tiver recusado antes (a pergunta voltaria a cada atualização).
  IfFileExists "$INSTDIR\vbcable\VBCABLE_Setup_x64.exe" 0 relay_vbcable_done
  ReadRegDWORD $0 HKLM "Software\Relay" "VbCableDeclined"
  StrCmp $0 "1" relay_vbcable_done
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -Command "if (Get-PnpDevice -FriendlyName '*VB-Audio Virtual Cable*' -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }"`
  Pop $0
  StrCmp $0 "0" relay_vbcable_done
  MessageBox MB_YESNO|MB_ICONQUESTION "Instalar o VB-CABLE para usar o microfone do Mac neste PC?$\r$\n$\r$\nO VB-CABLE é um driver de áudio da VB-Audio (vb-cable.com), distribuído como donationware: se ele for útil, considere doar ao autor.$\r$\n$\r$\nO Windows vai pedir para confirmar a instalação do driver. Reinicie o PC no fim." /SD IDNO IDYES relay_vbcable_install
  WriteRegDWORD HKLM "Software\Relay" "VbCableDeclined" 1
  Goto relay_vbcable_done
  relay_vbcable_install:
  ExecWait '"$INSTDIR\vbcable\VBCABLE_Setup_x64.exe" -i -h'
  relay_vbcable_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Para o serviço antes de o uninstall apagar o executável; erros são ignorados (já removido).
  nsExec::ExecToLog 'sc.exe stop RelayService'
  Pop $0
  nsExec::ExecToLog '"$INSTDIR\relay-service.exe" uninstall'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Relay"'
  Pop $0
!macroend
