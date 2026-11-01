"""Writes the "everyday" test set: short, plain sentences like the ones people
type to try a checker (names, home life, simple topics), each with one natural
slip, paired with its correction. The corrections are the false-alarm test.
Hand-written for Xenify (MIT); the first ones are the user's own examples.

    python3 everyday_set.py OUT.jsonl
"""
import json, sys

# (kind, with the mistake, corrected)
PAIRS = [
    # The user's own tests.
    ("agreement", "Mr Monkey sit at the table enjoying pie.", "Mr Monkey sits at the table enjoying pie."),
    ("sound_alike", "Mr Monkey sits at the table enjoying pi.", "Mr Monkey sits at the table enjoying pie."),
    # Reported 2026-10-19 in the HTML Write.
    ("missing_word", "I sat my chair.", "I sat on my chair."),
    ("missing_word", "Can you pass me salt?", "Can you pass me the salt?"),
    ("missing_word", "I went store yesterday.", "I went to the store yesterday."),
    ("run_on", "I went home I ate dinner.", "I went home. I ate dinner."),
    ("run_on", "Mum cooked dinner we watched a film.", "Mum cooked dinner. We watched a film."),
    ("run_on", "She was tired she went to bed early.", "She was tired. She went to bed early."),
    ("sound_alike", "There we’re fascinating pastries at the end of the table.", "There were fascinating pastries at the end of the table."),
    ("garbled", "Challenge the grammar do things.", "Challenge the grammar to do things."),
    ("typo", "New ways off working.", "New ways of working."),
    # Agreement with names and nouns as the subject.
    ("agreement", "Bob play games with the children every Sunday.", "Bob plays games with the children every Sunday."),
    ("agreement", "My brother live in Perth with his wife.", "My brother lives in Perth with his wife."),
    ("agreement", "Sarah don't like spicy food.", "Sarah doesn't like spicy food."),
    ("agreement", "The dog bark at the postman every morning.", "The dog barks at the postman every morning."),
    ("agreement", "Grandma make the best scones in town.", "Grandma makes the best scones in town."),
    ("agreement", "The kids was playing in the garden.", "The kids were playing in the garden."),
    ("agreement", "Tom and Lucy is coming for dinner.", "Tom and Lucy are coming for dinner."),
    ("agreement", "Everyone in the class were happy.", "Everyone in the class was happy."),
    ("agreement", "My parents has a small farm near Ballarat.", "My parents have a small farm near Ballarat."),
    ("agreement", "The cat sleep on my bed every night.", "The cat sleeps on my bed every night."),
    ("agreement", "Our teacher give us too much homework.", "Our teacher gives us too much homework."),
    ("agreement", "Mrs Patel walk to the shops each morning.", "Mrs Patel walks to the shops each morning."),
    ("agreement", "The flowers in the vase smells lovely.", "The flowers in the vase smell lovely."),
    ("agreement", "There is three apples left in the bowl.", "There are three apples left in the bowl."),
    ("agreement", "He don't know the answer.", "He doesn't know the answer."),
    ("agreement", "Jack have a red bicycle.", "Jack has a red bicycle."),
    # Sound-alikes.
    ("sound_alike", "Their going to the beach tomorrow.", "They're going to the beach tomorrow."),
    ("sound_alike", "I left my keys over their.", "I left my keys over there."),
    ("sound_alike", "Your welcome to stay the night.", "You're welcome to stay the night."),
    ("sound_alike", "The dog wagged it's tail.", "The dog wagged its tail."),
    ("sound_alike", "Its raining again today.", "It's raining again today."),
    ("sound_alike", "She is taller then her sister.", "She is taller than her sister."),
    ("sound_alike", "I would rather walk then drive.", "I would rather walk than drive."),
    ("sound_alike", "Don't loose your ticket.", "Don't lose your ticket."),
    ("sound_alike", "We went too the park.", "We went to the park."),
    ("sound_alike", "I ate to much cake.", "I ate too much cake."),
    ("sound_alike", "Can you here the music?", "Can you hear the music?"),
    ("sound_alike", "I new the answer straight away.", "I knew the answer straight away."),
    ("sound_alike", "We need to brake the chocolate into pieces.", "We need to break the chocolate into pieces."),
    ("sound_alike", "Please right your name at the top.", "Please write your name at the top."),
    ("sound_alike", "He through the ball over the fence.", "He threw the ball over the fence."),
    ("sound_alike", "I'll meat you outside the cinema.", "I'll meet you outside the cinema."),
    ("sound_alike", "We bought fresh flower to make bread.", "We bought fresh flour to make bread."),
    ("sound_alike", "I don't know weather it will rain.", "I don't know whether it will rain."),
    ("sound_alike", "The hole family came to the party.", "The whole family came to the party."),
    ("sound_alike", "She wore a beautiful blue dress to the ball and looked grate.", "She wore a beautiful blue dress to the ball and looked great."),
    ("sound_alike", "Can I have a peace of cake?", "Can I have a piece of cake?"),
    ("sound_alike", "The bare growled at the campers.", "The bear growled at the campers."),
    ("sound_alike", "We walked passed the old church.", "We walked past the old church."),
    ("sound_alike", "Who's coat is this?", "Whose coat is this?"),
    ("sound_alike", "I could of helped you.", "I could have helped you."),
    ("sound_alike", "We should of left earlier.", "We should have left earlier."),
    ("sound_alike", "The weather had a big affect on the crops.", "The weather had a big effect on the crops."),
    ("sound_alike", "I except your apology.", "I accept your apology."),
    ("sound_alike", "Turn write at the next corner.", "Turn right at the next corner."),
    ("sound_alike", "The children where very quiet.", "The children were very quiet."),
    ("sound_alike", "We we're late for school.", "We were late for school."),
    ("sound_alike", "Where going on holiday next week.", "We're going on holiday next week."),
    ("sound_alike", "The tail of the princess was read at bedtime.", "The tale of the princess was read at bedtime."),
    ("sound_alike", "I'm board of this game.", "I'm bored of this game."),
    ("sound_alike", "She has a soar throat.", "She has a sore throat."),
    ("sound_alike", "We saw a dear in the forest.", "We saw a deer in the forest."),
    ("sound_alike", "Wait hear while I get the car.", "Wait here while I get the car."),
    # Missing or extra words.
    ("missing_word", "I going to the shops.", "I am going to the shops."),
    ("missing_word", "She very happy today.", "She is very happy today."),
    ("missing_word", "We went to beach on Saturday.", "We went to the beach on Saturday."),
    ("missing_word", "Can you pass me salt?", "Can you pass me the salt?"),
    ("missing_word", "He wants go home now.", "He wants to go home now."),
    ("missing_word", "I have been waiting for hour.", "I have been waiting for an hour."),
    ("missing_word", "My mum is doctor.", "My mum is a doctor."),
    ("missing_word", "They arrived the airport late.", "They arrived at the airport late."),
    ("missing_word", "Lily listened the radio all afternoon.", "Lily listened to the radio all afternoon."),
    ("missing_word", "Do you want come with us?", "Do you want to come with us?"),
    ("missing_word", "Max is best player on the team.", "Max is the best player on the team."),
    ("extra_word", "I am agree with you.", "I agree with you."),
    ("extra_word", "We discussed about the plan.", "We discussed the plan."),
    ("extra_word", "She married with a farmer.", "She married a farmer."),
    ("extra_word", "He entered into the room quietly.", "He entered the room quietly."),
    ("extra_word", "I will call to you tomorrow.", "I will call you tomorrow."),
    ("extra_word", "The the cat is asleep.", "The cat is asleep."),
    ("extra_word", "My three dogs dogs bark at night.", "My three dogs bark at night."),
    # Verb forms and tense.
    ("verb_form", "I seen him at the shop.", "I saw him at the shop."),
    ("verb_form", "She has went home already.", "She has gone home already."),
    ("verb_form", "We goed to the zoo yesterday.", "We went to the zoo yesterday."),
    ("verb_form", "He brung his guitar to the party.", "He brought his guitar to the party."),
    ("verb_form", "I have ate all the biscuits.", "I have eaten all the biscuits."),
    ("verb_form", "They was singing in the car.", "They were singing in the car."),
    ("verb_form", "Yesterday I walk to school.", "Yesterday I walked to school."),
    ("verb_form", "Last week we buy a new car.", "Last week we bought a new car."),
    ("verb_form", "She can sings very well.", "She can sing very well."),
    ("verb_form", "Did you finished your homework?", "Did you finish your homework?"),
    ("verb_form", "He doesn't likes broccoli.", "He doesn't like broccoli."),
    ("verb_form", "I am looking forward to see you.", "I am looking forward to seeing you."),
    ("verb_form", "The window was broke by the storm.", "The window was broken by the storm."),
    ("verb_form", "I have knew her for years.", "I have known her for years."),
    ("verb_form", "We was going to call you.", "We were going to call you."),
    # Plurals, comparatives, pronouns.
    ("plural", "I have two cat and a dog.", "I have two cats and a dog."),
    ("plural", "The childs played in the park.", "The children played in the park."),
    ("plural", "We saw many sheeps on the farm.", "We saw many sheep on the farm."),
    ("plural", "There are five mouses in the barn.", "There are five mice in the barn."),
    ("plural", "She bought three loaf of bread.", "She bought three loaves of bread."),
    ("comparative", "This cake is more better than that one.", "This cake is better than that one."),
    ("comparative", "He is the most tallest boy in class.", "He is the tallest boy in class."),
    ("comparative", "Today is more hotter than yesterday.", "Today is hotter than yesterday."),
    ("comparative", "My bag is more heavy than yours.", "My bag is heavier than yours."),
    ("pronoun", "Me and Sam went fishing.", "Sam and I went fishing."),
    ("pronoun", "Him and his dad built a treehouse.", "He and his dad built a treehouse."),
    ("pronoun", "Give the book to Sarah and I.", "Give the book to Sarah and me."),
    ("pronoun", "Between you and I, the cake was dry.", "Between you and me, the cake was dry."),
    ("pronoun", "Her and I are best friends.", "She and I are best friends."),
    # Articles.
    ("article", "I ate a apple for lunch.", "I ate an apple for lunch."),
    ("article", "She is an good friend.", "She is a good friend."),
    ("article", "We waited for an hour and a half at a airport.", "We waited for an hour and a half at an airport."),
    ("article", "He wants to be a engineer.", "He wants to be an engineer."),
    # Word order and questions.
    ("word_order", "Where you are going?", "Where are you going?"),
    ("word_order", "What time it is?", "What time is it?"),
    ("word_order", "I always am late on Mondays.", "I am always late on Mondays."),
    ("word_order", "Can you tell me where is the station?", "Can you tell me where the station is?"),
    ("word_order", "She plays well very the piano.", "She plays the piano very well."),
    # Doesn't make sense.
    ("garbled", "We the movie to watch on Friday.", "We went to watch the movie on Friday."),
    ("garbled", "The dog ran to the park the.", "The dog ran to the park."),
    ("garbled", "My sister bought a new car blue very.", "My sister bought a very nice blue car."),
    ("garbled", "I think that the because rain stopped.", "I think that the rain stopped."),
    ("garbled", "Mum asked me to the dishes after tea.", "Mum asked me to do the dishes after tea."),
    ("garbled", "He went the shop to buy some milk at.", "He went to the shop to buy some milk."),
    ("garbled", "The teacher said that we must our books.", "The teacher said that we must bring our books."),
    ("garbled", "Grandpa the newspaper every morning reads.", "Grandpa reads the newspaper every morning."),
]

# Correct sentences of the same kind, which should stay unflagged.
CLEAN = [
    # Seen in Xenify Write's own sample (2026-10-18): nouns Harper took for verbs.
    "Checkout redesign",
    "She sat the chair down by the window.",
    "I liked the book I read.",
    "I think he left early.",
    "I told him she was here.",
    "The day I met you was great.",
    "I was glad they came.",
    "Please pass me something to write with.",
    "He sat my sister on the bench.",
    "Can you pass me the salt?",
    "There is a lot to do this week.",
    "This project will streamline checkout to a single page.",
    "The setup takes five minutes and the login page is next.",
    "Bob plays games with children.",
    "Women's dance in the sunset.",
    "Mr Monkey sat quietly by the window.",
    "Lucy and her friends baked a cherry pie.",
    "The children were very quiet during the film.",
    "There were three cakes on the plate.",
    "We're going to the beach on Saturday.",
    "Their house is at the end of the street.",
    "It's a lovely day for a picnic.",
    "The cat licked its paws.",
    "I would rather read than watch television.",
    "Grandpa reads the newspaper every morning.",
    "We sat at the table and talked for hours.",
    "Pi is a little more than three.",
    "Can I have a piece of pie?",
    "She threw the ball through the window.",
    "The whole class went on the trip.",
    "I knew you would like the new book.",
    "Mrs Patel grows tomatoes in her garden.",
    "Sam and I went fishing last weekend.",
    "The dog barked at the postman.",
    "He had had enough of the noise.",
    "That was a very, very long day.",
    "Max scored the winning goal.",
    "My brother lives in Perth.",
    "Everyone in the class was happy.",
]


def changed_span(wrong, right):
    """The words of `wrong` that differ from `right` (shared start and end trimmed)."""
    a, b = wrong.split(" "), right.split(" ")
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]:
        i += 1
    j = 0
    while j < min(len(a), len(b)) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    lo, hi = i, len(a) - j
    if lo == hi:  # a word was left out: mark the words either side
        lo, hi = max(0, i - 1), min(len(a), i + 1)
    # A short span ("a") also takes the next word, so it's found as a whole word.
    if len(" ".join(a[lo:hi])) < 4 and hi < len(a):
        hi += 1
    wa, wb = a[lo:hi], b[lo : hi - len(a) + len(b)]
    return " ".join(wa).strip(".?!,"), " ".join(wb).strip(".?!,")


out = []
for n, (kind, wrong, right) in enumerate(PAIRS):
    w, f = changed_span(wrong, right)
    out.append({
        "id": f"everyday:{kind}:{n}", "job": f"everyday:{kind}", "setting": "everyday", "register": "standard",
        "error_type": kind, "text": wrong, "corrected": right, "mistakes": [{"wrong": w, "fix": f}],
        "generator": "hand-written",
    })
for n, t in enumerate(CLEAN):
    out.append({
        "id": f"everyday:clean:{n}", "job": "everyday:clean", "setting": "everyday", "register": "standard",
        "error_type": None, "text": t, "corrected": t, "mistakes": [], "generator": "hand-written",
    })
with open(sys.argv[1], "w", encoding="utf-8") as fh:
    for r in out:
        fh.write(json.dumps(r, ensure_ascii=False) + "\n")
print(f"{len(PAIRS)} sentences with a mistake, {len(CLEAN)} correct → {sys.argv[1]}")
