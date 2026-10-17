package applepeer

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"golang.org/x/sys/unix"
	"net"
	"os"
)

func checkUID(conn *net.UnixConn) error {
	raw, err := conn.SyscallConn()
	if err != nil {
		return err
	}
	var peerErr error
	err = raw.Control(func(fd uintptr) {
		credential, e := unix.GetsockoptUcred(int(fd), unix.SOL_SOCKET, unix.SO_PEERCRED)
		peerErr = e
		if e == nil && credential.Uid != uint32(os.Getuid()) {
			peerErr = contracts.ErrUnauthorized
		}
	})
	if err != nil {
		return err
	}
	return peerErr
}
