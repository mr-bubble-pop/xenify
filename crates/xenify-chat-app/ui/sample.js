// Sample people and chats, in the chat server's shapes, used until the app
// is signed in to a server (and for screenshots).

const now = Date.now();
const min = 60_000;
const day = 24 * 60 * min;
const user = (id, username, display_name) => ({ id, username, display_name, app: false });

const alex = user(1, "alex", "Alex Lane");
const priya = user(2, "priya", "Priya Nair");
const sam = user(3, "sam", "Sam Lee");
const jordan = user(4, "jordan", "Jordan Ortiz");

let next = 1;
const msg = (conv, seq, sender, text, at) => ({ id: next++, conv, seq, sender, text, at, client_id: "", edited: false, deleted: false });

const messages = {
  1: [
    msg(1, 1, priya, "Morning! The draft of the onboarding flow is in the shared folder.", now - 40 * min),
    msg(1, 2, priya, "Would love a second pair of eyes on the sign-in screen before lunch.", now - 39 * min),
    msg(1, 3, alex, "On it. It looks clean so far.", now - 33 * min),
    msg(1, 4, alex, "One thing: the password hint feels a bit long.", now - 32 * min),
    msg(1, 5, sam, "Agree, let's trim it to one line.", now - 29 * min),
  ],
  2: [msg(2, 1, jordan, "Build #482 failed on main: 3 tests in checkout.", now - 66 * min)],
  3: [msg(3, 1, sam, "Can you send the slides?", now - 96 * min)],
  4: [msg(4, 1, alex, "Here are my notes on the copy.", now - day - 120 * min), msg(4, 2, priya, "Thanks, that helps a lot.", now - day - 100 * min)],
  5: [msg(5, 1, jordan, "Moved to Thursday at 2 pm.", now - 3 * day)],
  6: [msg(6, 1, alex, "Buy printer paper", now - 9 * day)],
};

const conv = (id, kind, title, members, extra = {}) => {
  const m = messages[id] || [];
  const last = m[m.length - 1] || null;
  return {
    id, kind, title, members, member_count: members.length, role: "member", can_post: kind !== "channel", open: false,
    last, last_seq: last ? last.seq : 0, read_seq: last ? last.seq : 0, notify: "all", muted_until: null, ...extra,
  };
};

// A few reactions to show how they look.
messages[1][3].reactions = [{ emoji: "👍", users: [priya, sam] }];
messages[1][4].reactions = [{ emoji: "🙏", users: [alex] }];

export const sample = {
  me: alex,
  conversations: [
    conv(1, "group", "Design team", [alex, priya, sam], { read_seq: 4, role: "owner" }),
    conv(2, "channel", "Build alerts", [jordan, alex, priya, sam], { read_seq: 0, member_count: 24, open: true }),
    conv(3, "direct", "", [alex, sam]),
    conv(4, "direct", "", [alex, priya]),
    conv(5, "group", "Weekly planning", [alex, jordan, priya, sam], { read_seq: 0, notify: "off" }),
    conv(6, "direct", "", [alex]),
  ],
  messages,
};
