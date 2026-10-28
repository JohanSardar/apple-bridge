package contracts

import "strings"

type MailArgs struct {
	ID          string   `json:"id,omitempty"`
	To          []string `json:"to,omitempty"`
	CC          []string `json:"cc,omitempty"`
	Subject     string   `json:"subject,omitempty"`
	Body        string   `json:"body,omitempty"`
	Attachments []string `json:"attachments,omitempty"`
	ReplyTo     string   `json:"reply_to,omitempty"`
	ForwardOf   string   `json:"forward_of,omitempty"`
	Mailbox     string   `json:"mailbox,omitempty"`
	Account     string   `json:"account,omitempty"`
	Read        *bool    `json:"read,omitempty"`
	Flagged     *bool    `json:"flagged,omitempty"`
}

func ValidateMailCommand(c Command) (ReminderArgs, error) {
	var a MailArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ReminderArgs{}, ErrInvalid
	}
	switch c.Op {
	case "mail.create_draft", "mail.send":
		if len(a.To) == 0 || a.Subject == "" {
			return ReminderArgs{}, ErrInvalid
		}
		for _, address := range append(a.To, a.CC...) {
			if !strings.Contains(address, "@") || strings.ContainsAny(address, "\r\n") {
				return ReminderArgs{}, ErrInvalid
			}
		}
		if len(a.Attachments) > 0 || a.ReplyTo != "" || a.ForwardOf != "" {
			return ReminderArgs{}, ErrUnsupported
		}
	case "mail.move":
		if a.ID == "" || a.Mailbox == "" || a.Account == "" {
			return ReminderArgs{}, ErrInvalid
		}
	case "mail.set_flag":
		if a.ID == "" || a.Flagged == nil {
			return ReminderArgs{}, ErrInvalid
		}
	case "mail.mark_read":
		if a.ID == "" || a.Read == nil {
			return ReminderArgs{}, ErrInvalid
		}
	case "mail.delete":
		if a.ID == "" {
			return ReminderArgs{}, ErrInvalid
		}
	default:
		return ReminderArgs{}, ErrUnsupported
	}
	return ReminderArgs{ID: a.ID}, nil
}
